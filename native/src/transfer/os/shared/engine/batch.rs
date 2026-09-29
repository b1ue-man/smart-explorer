//! Packets of small files for peers that speak a batch protocol (Share hosts,
//! the SSH agent): one round trip for many files. The packet size follows
//! the measured rate (≈ 250 ms per packet) within the backend's limits.
//! Uploads stream the files one after another from disk into `put_batch`
//! (nothing is buffered first, K7); an ambiguous packet failure leaves every
//! file of it "result unknown" and nothing is retried blindly.
use super::super::engine_names::{base_name, parent_rel};
use super::super::engine_policy::is_transient;
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::super::memory::reserve_memory;
use super::ops::{At, Meter, OpError};
use super::queue::{FileWork, Work};
use super::source::LocalSource;
use super::view::Side;
use super::worker::{failed, finished, parent_ready, run_file};
use super::Engine;
use crate::vfs::{Backend, BatchPut, BatchPutOutcome};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// More small files for a packet with `file`, or `None` for a single
/// transfer (no batch support, a large file, nothing else small queued).
pub(super) fn members(engine: &Engine<'_>, file: &FileWork) -> Option<Vec<FileWork>> {
    let limits = engine.batch?;
    if file.retried || engine.resume {
        return None;
    }
    let (target, small) = {
        let sizer = super::lock(&engine.sizer);
        (
            sizer.target_bytes(limits.max_bytes),
            sizer.small_file_limit(limits.max_bytes),
        )
    };
    if file.size > small {
        return None;
    }
    let more = engine.queue.take_small(
        limits.max_files.saturating_sub(1),
        target.saturating_sub(file.size),
        small,
    );
    (!more.is_empty()).then_some(more)
}

/// Runs one packet of `first` and `members`.
pub(super) fn run(engine: &Engine<'_>, first: FileWork, members: Vec<FileWork>, buffer: &mut [u8]) {
    let files: Vec<FileWork> = std::iter::once(first)
        .chain(members)
        .filter(|file| parent_ready(engine, file).is_some())
        .collect();
    match (engine.view.source, engine.view.target) {
        (Side::Local, Side::Remote(target)) => upload(engine, target, files, buffer),
        (Side::Remote(source), Side::Local) => {
            super::batch_get::download(engine, source, files, buffer)
        }
        _ => {
            for file in files {
                run_file(engine, file, buffer);
            }
        }
    }
}

/// A failed packet member: retried alone once when that is safe, else
/// reported.
pub(super) fn member_failed(engine: &Engine<'_>, mut file: FileWork, failure: OpError) {
    if engine.stopped() {
        return;
    }
    let overload = classify_error(&failure.error) == OpOutcome::Overload;
    let before_publication = matches!(failure.at, At::Source | At::Target);
    if before_publication && !file.retried && is_transient(failure.error.kind(), overload) {
        file.retried = true;
        engine.queue.push_front(Work::File(file));
        return;
    }
    if failure.at == At::Target
        && super::super::engine_policy::ends_job_at_target(failure.error.kind())
    {
        engine.fatal(format!(
            "Das Ziel nimmt keine Dateien mehr an – Übertragung beendet: {}",
            failure.error
        ));
        return;
    }
    failed(engine, &file.source, &failure);
}

fn upload(engine: &Engine<'_>, target: &dyn Backend, files: Vec<FileWork>, buffer: &mut [u8]) {
    let mut opened = Vec::with_capacity(files.len());
    for file in files {
        match LocalSource::open(&file.source) {
            Ok(source) => opened.push((file, source)),
            // Alone again: that path asks for access or reports the reason.
            Err(_) => run_file(engine, file, buffer),
        }
    }
    if opened.len() < 2 {
        for (file, source) in opened {
            drop(source);
            run_file(engine, file, buffer);
        }
        return;
    }
    let bytes: u64 = opened.iter().map(|(_, source)| source.length()).sum();
    let Some(reservation) = reserve_memory(bytes, engine.stop_flag()) else {
        return;
    };
    let Some(permits) = engine.acquire() else {
        return;
    };
    let active = engine
        .stats
        .begin(&format!("{} kleine Dateien (Paket)", opened.len()));
    engine.queue.notify();
    let entries: Vec<BatchPut> = opened
        .iter()
        .map(|(file, source)| BatchPut {
            path: engine.folders.path_of(&file.rel),
            size: source.length(),
        })
        .collect();
    let meter = Meter::new(&permits, &engine.stats);
    let started = Instant::now();
    let result = {
        let mut data = PacketData {
            sources: opened.iter_mut().map(|(_, source)| source).collect(),
            index: 0,
            sent: 0,
            meter: &meter,
            stop: engine.stop_flag(),
        };
        target.put_batch(&entries, &mut data)
    };
    let moved = meter.moved();
    permits.finish(match &result {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    let outcomes = match result {
        Ok(outcomes) => outcomes,
        // Refused before anything was sent: each file goes alone.
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            engine.stats.unmoved(moved);
            for (file, source) in opened {
                drop(source);
                run_file(engine, file, buffer);
            }
            return;
        }
        Err(error) => {
            engine.stats.unmoved(moved);
            if engine.stopped() {
                return;
            }
            let message = format!("Paket-Upload: {error}");
            for (file, _) in opened {
                failed(
                    engine,
                    &file.source,
                    &OpError::at(At::Unknown, io::Error::new(error.kind(), message.clone())),
                );
            }
            return;
        }
    };
    let mut outcomes = outcomes.into_iter();
    for (file, source) in opened {
        match outcomes.next() {
            Some(BatchPutOutcome::Published(path)) => match source.verify() {
                Ok(()) => {
                    if parent_rel(&file.rel).is_none() {
                        engine.folders.record_alias(&file.rel, base_name(&path));
                    }
                    finished(engine, super::ops::Outcome::Done);
                }
                Err(changed) => failed(
                    engine,
                    &file.source,
                    &OpError::at(
                        At::Unknown,
                        io::Error::new(
                            changed.error.kind(),
                            format!("{}; „{path}“ wurde trotzdem angelegt", changed.error),
                        ),
                    ),
                ),
            },
            Some(BatchPutOutcome::Failed(error)) => {
                engine.stats.unmoved(source.length());
                member_failed(engine, file, OpError::target(error));
            }
            None => failed(
                engine,
                &file.source,
                &OpError::at(
                    At::Unknown,
                    io::Error::other("Paket-Upload lieferte kein Ergebnis für diese Datei"),
                ),
            ),
        }
    }
}

/// The files of a packet back to back, read straight from disk.
struct PacketData<'s, 'm> {
    sources: Vec<&'s mut LocalSource>,
    index: usize,
    sent: u64,
    meter: &'s Meter<'m>,
    stop: &'s AtomicBool,
}

impl Read for PacketData<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    super::super::cancel::CANCELED_ERROR,
                ));
            }
            let Some(source) = self.sources.get_mut(self.index) else {
                return Ok(0);
            };
            let remaining = source.length().saturating_sub(self.sent);
            if remaining == 0 {
                self.index += 1;
                self.sent = 0;
                continue;
            }
            if buffer.is_empty() {
                return Ok(0);
            }
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let read = source.file_mut().read(&mut buffer[..wanted])?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Quelle ist während der Übertragung geschrumpft",
                ));
            }
            self.sent += read as u64;
            self.meter.add(read as u64);
            return Ok(read);
        }
    }
}
