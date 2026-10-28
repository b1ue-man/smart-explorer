//! Streaming hydration of a clean spool (G4 ML4).
//!
//! The first data access no longer waits for the whole download: the entry
//! is installed at once, a `mount-hydrate` thread fills its spool in order,
//! and a read waits only until its own range has arrived. Writes, truncation
//! and anything that needs the whole content wait for the end.
//!
//! One version only: the download checks the remote identity (id, size,
//! modification time, content hash where known) when it starts, after every
//! `CHECK_EVERY` bytes and at the end. A change fails the hydration: every
//! read through a handle of this entry reports an error from then on, the
//! entry is dropped from the path index, and the next open downloads the new
//! version into a new spool. Bytes of the old version that a read returned
//! before the change was seen cannot be taken back; bytes of two versions
//! are never mixed in one spool or one handle.
//!
//! Clean data needs no `fdatasync`: a clean spool is never journaled, so a
//! crash only leaves an orphan the next start removes.
use super::cache_space::GrowthReservation;
use super::engine::{baseline_from_meta, lock, require_regular, Entry};
use super::spool_storage::SpoolHandle;
use super::types::Baseline;
use crate::vfs::{BackendHandle, VfsMeta};
use std::io::{self, Read, Write};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex};

/// Files up to this size download whole before the first read, as before:
/// one block arrives in about one round trip, so streaming gains nothing,
/// and the handle keeps its snapshot of the content at open.
pub(super) const STREAM_ABOVE: u64 = 1024 * 1024;
/// Bytes between two identity checks while a download runs.
const CHECK_EVERY: u64 = 64 * 1024 * 1024;
/// Bytes per read from the backend and write into the spool.
const BLOCK: usize = 1024 * 1024;

pub(super) struct Hydration {
    total: u64,
    progress: Mutex<Progress>,
    moved: Condvar,
}

#[derive(Default)]
struct Progress {
    available: u64,
    outcome: Option<Result<(), Failure>>,
}

#[derive(Clone)]
struct Failure {
    kind: io::ErrorKind,
    detail: String,
}

impl Failure {
    fn error(&self) -> io::Error {
        io::Error::new(self.kind, self.detail.clone())
    }
}

impl Hydration {
    pub(super) fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            total,
            progress: Mutex::new(Progress::default()),
            moved: Condvar::new(),
        })
    }

    /// The length of the content being downloaded.
    pub(super) fn total(&self) -> u64 {
        self.total
    }

    /// The download ended with the whole, unchanged content.
    pub(super) fn succeeded(&self) -> bool {
        self.progress
            .lock()
            .is_ok_and(|progress| matches!(progress.outcome, Some(Ok(()))))
    }

    /// The download ended, with or without a usable spool.
    pub(super) fn ended(&self) -> bool {
        self.progress
            .lock()
            .is_ok_and(|progress| progress.outcome.is_some())
    }

    /// The download ended without a usable spool.
    pub(super) fn failed(&self) -> bool {
        self.progress
            .lock()
            .is_ok_and(|progress| matches!(progress.outcome, Some(Err(_))))
    }

    /// Waits until the bytes before `end` are in the spool. A read up to the
    /// end of the content also waits for the final identity check, so no
    /// reader completes a file whose version changed meanwhile. An ended
    /// download answers at once: its failure as an error.
    pub(super) fn wait_for(&self, end: u64) -> io::Result<()> {
        if end >= self.total {
            return self.wait_complete();
        }
        let mut progress = lock(&self.progress)?;
        loop {
            match &progress.outcome {
                Some(Err(failure)) => return Err(failure.error()),
                Some(Ok(())) => return Ok(()),
                None if progress.available >= end => return Ok(()),
                None => {}
            }
            progress = self.moved.wait(progress).map_err(|_| poisoned())?;
        }
    }

    /// Waits for the end of the download; `Ok` only for the whole content.
    pub(super) fn wait_complete(&self) -> io::Result<()> {
        let mut progress = lock(&self.progress)?;
        loop {
            match &progress.outcome {
                Some(Err(failure)) => return Err(failure.error()),
                Some(Ok(())) => return Ok(()),
                None => {}
            }
            progress = self.moved.wait(progress).map_err(|_| poisoned())?;
        }
    }

    fn advance(&self, available: u64) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.available = available;
        }
        self.moved.notify_all();
    }

    pub(super) fn finish(&self, outcome: io::Result<()>) {
        if let Ok(mut progress) = self.progress.lock() {
            if progress.outcome.is_none() {
                progress.outcome = Some(outcome.map_err(|error| Failure {
                    kind: error.kind(),
                    detail: error.to_string(),
                }));
            }
        }
        self.moved.notify_all();
    }
}

/// A download prepared by the materialization; it starts once its entry is
/// installed, so the thread holds that entry until the spool is complete.
pub(super) struct Download {
    pub(super) hydration: Arc<Hydration>,
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) spool: SpoolHandle,
    pub(super) backend: BackendHandle,
    pub(super) meta: VfsMeta,
    pub(super) growth: GrowthReservation,
}

impl Download {
    /// Runs the download on its own thread. The entry cannot retire while
    /// its hydration runs (`retire_entry`); its end schedules the retirement.
    pub(super) fn start(self, entry: Arc<Entry>) {
        let hydration = Arc::clone(&self.hydration);
        let spawned = std::thread::Builder::new()
            .name("mount-hydrate".into())
            .spawn(move || {
                let hydration = Arc::clone(&self.hydration);
                // A panic must still end the hydration: readers wait for it.
                let run = std::panic::catch_unwind(AssertUnwindSafe(|| self.run(&entry)));
                let outcome = run.unwrap_or_else(|_| Err(ended_unexpectedly()));
                hydration.finish(outcome);
                entry.schedule_retirement();
            });
        if let Err(error) = spawned {
            hydration.finish(Err(error));
        }
    }

    fn run(self, entry: &Arc<Entry>) -> io::Result<()> {
        let Self {
            hydration,
            mut reader,
            mut spool,
            backend,
            meta,
            growth,
        } = self;
        let expected = baseline_from_meta(&meta);
        let mut buffer = vec![0; BLOCK];
        let mut copied = 0u64;
        let mut next_check = CHECK_EVERY;
        loop {
            // One byte more than advertised proves a longer file.
            let left = hydration.total().saturating_add(1).saturating_sub(copied);
            let wanted = left.min(BLOCK as u64) as usize;
            let read = match reader.read(&mut buffer[..wanted]) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if copied.saturating_add(read as u64) > hydration.total() {
                return Err(longer());
            }
            spool.write_all(&buffer[..read])?;
            copied += read as u64;
            hydration.advance(copied);
            if copied >= next_check && copied < hydration.total() {
                // The reader holds a request slot; release it before the
                // check and continue at the same offset afterwards.
                drop(reader);
                verify(&backend, &current_path(entry)?, &expected)?;
                reader = reopen(&backend, &current_path(entry)?, &meta, copied)?;
                next_check = copied.saturating_add(CHECK_EVERY);
            }
        }
        drop(reader);
        if copied != hydration.total() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "remote content length differs from its advertised size",
            ));
        }
        verify(&backend, &current_path(entry)?, &expected)?;
        drop(growth);
        Ok(())
    }
}

/// The entry's current backend path: an own rename moves the entry.
fn current_path(entry: &Arc<Entry>) -> io::Result<String> {
    Ok(lock(&entry.state)?.remote_path.clone())
}

fn verify(backend: &BackendHandle, path: &str, expected: &Baseline) -> io::Result<()> {
    let fresh = backend.stat(path)?;
    require_regular(&fresh)?;
    if baseline_from_meta(&fresh) != *expected {
        return Err(changed());
    }
    Ok(())
}

/// A reader at `offset`; without range support the stream is read again
/// from the start and the copied bytes are skipped.
fn reopen(
    backend: &BackendHandle,
    path: &str,
    meta: &VfsMeta,
    offset: u64,
) -> io::Result<Box<dyn Read + Send>> {
    if let Some(reader) = backend.open_read_at(path, meta.id.as_deref(), offset)? {
        return Ok(reader);
    }
    let mut reader = backend.open_read_id(path, meta.id.as_deref())?;
    let skipped = io::copy(&mut (&mut reader).take(offset), &mut io::sink())?;
    if skipped != offset {
        return Err(changed());
    }
    Ok(reader)
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "remote file changed while it was being read; reopen it to read the new version",
    )
}

fn longer() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "remote content is longer than its advertised size",
    )
}

fn ended_unexpectedly() -> io::Error {
    io::Error::other("mounted download ended unexpectedly")
}

fn poisoned() -> io::Error {
    io::Error::other("mounted download progress lock is poisoned")
}

#[cfg(test)]
#[path = "hydration_task_tests.rs"]
mod task_tests;
