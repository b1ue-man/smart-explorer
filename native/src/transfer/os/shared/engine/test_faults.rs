//! Faults of the fake remote beyond plain reads and writes: packets as the
//! agent answers them (a failed source read ends a packet, the entries after
//! it fail as "not sent"; an item whose length changed since the listing is
//! not sent), refusals as congestion, lost answers, and a full target that
//! refuses fresh uploads when they complete.
use super::test_backend::Fake;
use crate::vfs::{Backend, BatchGet, BatchPut, BatchPutOutcome, BatchSink, VfsResult};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Runs with the first entry's path of the next packet, before its bytes
/// are read.
pub(super) type EntryHook = Box<dyn FnOnce(&str) + Send>;

#[derive(Default)]
pub(super) struct Faults {
    /// Stage opens refused as congestion before one succeeds.
    pub congested_stages: AtomicUsize,
    /// Folder creations refused as congestion before one succeeds.
    pub congested_dirs: AtomicUsize,
    /// Fresh uploads fail with this kind when they complete.
    pub fresh_refused: Option<io::ErrorKind>,
    /// Folder creations fail with this kind; nothing is created.
    pub dir_fault: Option<io::ErrorKind>,
    /// The next exclusive folder creation happens, then its answer is lost.
    pub dir_answer_lost: AtomicBool,
    /// Every packet publishes all entries, then its answer is lost.
    pub packet_answer_lost: bool,
    /// The first entry of the next packet is published, then reported as
    /// failed with this kind.
    pub failed_after_publish: Mutex<Option<io::ErrorKind>>,
    pub before_first_entry: Mutex<Option<EntryHook>>,
    /// The first item of the next download packet grows before it is read
    /// (its path is kept in `grown`).
    pub grow_first_item: AtomicBool,
    pub grown: Mutex<Option<String>>,
    /// This file gets other bytes of the same length and a later time before
    /// the next server copy.
    pub rewrite_before_copy: Mutex<Option<String>>,
}

impl Faults {
    /// One more congestion refusal of a stage open, if any are left.
    pub(super) fn refuse_stage(&self) -> Option<io::Error> {
        refuse(&self.congested_stages)
    }

    /// A folder creation refused (congestion) or failing as configured.
    pub(super) fn refuse_dir(&self) -> Option<io::Error> {
        refuse(&self.congested_dirs).or_else(|| {
            self.dir_fault
                .map(|kind| io::Error::new(kind, "fixture: folder refused"))
        })
    }
}

fn refuse(left: &AtomicUsize) -> Option<io::Error> {
    left.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
        count.checked_sub(1)
    })
    .ok()
    .map(|_| {
        crate::vfs::congestion_error(
            "fixture: zu viele Anfragen",
            Some(Duration::from_millis(10)),
        )
    })
}

/// A fresh upload the target refuses when it completes.
pub(super) struct RefusedFresh(pub io::ErrorKind);

impl Write for RefusedFresh {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(self.0, "fixture: fresh upload refused"))
    }
}

/// Other bytes of the same length and a modification time 5 s later.
pub(super) fn rewrite_same_length(path: &str) -> io::Result<()> {
    let mut bytes = std::fs::read(path)?;
    for byte in &mut bytes {
        *byte = byte.wrapping_add(1);
    }
    let modified = std::fs::metadata(path)?.modified()?;
    std::fs::write(path, &bytes)?;
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(modified + Duration::from_secs(5))
}

impl Fake {
    pub(super) fn fake_put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        self.counters.put_batches.fetch_add(1, Ordering::SeqCst);
        let hook = self.faults.before_first_entry.lock().expect("hook").take();
        if let (Some(hook), Some(first)) = (hook, entries.first()) {
            hook(&first.path);
        }
        let mut failed_after = self
            .faults
            .failed_after_publish
            .lock()
            .expect("fault")
            .take();
        let mut outcomes = Vec::new();
        let mut stopped = false;
        for (index, entry) in entries.iter().enumerate() {
            if stopped {
                outcomes.push(BatchPutOutcome::Failed(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("Nicht übertragen (Eintrag {})", index + 1),
                )));
                continue;
            }
            if self.batch_ambiguous && index == 1 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "fixture: packet answer lost",
                ));
            }
            self.counters
                .batched
                .lock()
                .expect("fixture lock")
                .push(entry.path.clone());
            let mut bytes = vec![0u8; entry.size as usize];
            if let Err(error) = data.read_exact(&mut bytes) {
                // As the agent: this entry is discarded, the rest not sent.
                outcomes.push(BatchPutOutcome::Failed(error));
                stopped = true;
                continue;
            }
            outcomes.push(
                match (self.publish_entry(&entry.path, &bytes), failed_after.take()) {
                    (BatchPutOutcome::Published(_), Some(kind)) => BatchPutOutcome::Failed(
                        io::Error::new(kind, "fixture: published, reported as failed"),
                    ),
                    (outcome, _) => outcome,
                },
            );
        }
        if self.faults.packet_answer_lost {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "fixture: packet published, answer lost",
            ));
        }
        Ok(outcomes)
    }

    fn publish_entry(&self, path: &str, bytes: &[u8]) -> BatchPutOutcome {
        let stage = format!("{path}.fixture-stage");
        let written = self.inner.open_write_new(&stage).and_then(|mut writer| {
            writer.write_all(bytes)?;
            writer.flush()
        });
        let published = written.and_then(|()| self.inner.promote_staged_no_replace(&stage, path));
        match published {
            Ok(()) => BatchPutOutcome::Published(path.to_string()),
            Err(error) => {
                let _ = std::fs::remove_file(&stage);
                BatchPutOutcome::Failed(error)
            }
        }
    }

    pub(super) fn fake_get_batch(
        &self,
        items: &[BatchGet],
        sink: &mut dyn BatchSink,
    ) -> VfsResult<()> {
        self.counters.get_batches.fetch_add(1, Ordering::SeqCst);
        if let Some(first) = items.first() {
            if self.faults.grow_first_item.swap(false, Ordering::SeqCst) {
                let mut file = std::fs::OpenOptions::new().append(true).open(&first.path)?;
                file.write_all(b" and more")?;
                *self.faults.grown.lock().expect("grown") = Some(first.path.clone());
            }
        }
        for (index, item) in items.iter().enumerate() {
            let bytes = match std::fs::read(Path::new(&item.path)) {
                Ok(bytes) => bytes,
                Err(error) => {
                    sink.failed(index, error)?;
                    continue;
                }
            };
            if bytes.len() as u64 != item.size {
                sink.failed(
                    index,
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Quelle wurde seit der Auflistung geändert ({} statt {} Bytes)",
                            bytes.len(),
                            item.size
                        ),
                    ),
                )?;
                continue;
            }
            sink.begin(index, bytes.len() as u64)?;
            for chunk in bytes.chunks(7) {
                sink.data(index, chunk)?;
            }
            sink.end(index, Ok(()))?;
        }
        Ok(())
    }
}
