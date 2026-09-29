//! Download packets: many small files in one `get_batch` round trip, written
//! straight into their `.part` files and published one by one under the
//! conflict policy as each completes. Local publication is exact, so a
//! failed packet leaves nothing ambiguous: unfinished files go alone again.
//! An item the peer did not send because it changed since the listing goes
//! alone with its current length; a target that takes nothing more ends the
//! job like a single download.
use super::super::engine_policy::{ends_job_at_target, listed_time_differs};
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::super::local_stage::ensure_local_space;
use super::super::memory::reserve_memory;
use super::super::walk_listers::native;
use super::batch::{Attempt, Failures};
use super::download::{existing, publish_local, Part};
use super::ops::{relabeled, Meter, OpError, OpResult};
use super::queue::FileWork;
use super::source::RemoteCheck;
use super::worker::{finished, run_file, target_refuses};
use super::Engine;
use crate::vfs::{Backend, BatchGet, BatchSink, VfsMeta};
use std::io;
use std::path::PathBuf;
use std::time::Instant;

pub(super) fn download(
    engine: &Engine<'_>,
    source: &dyn Backend,
    files: Vec<FileWork>,
    buffer: &mut [u8],
) {
    let mut members = Vec::with_capacity(files.len());
    let mut failures = Failures::new(engine);
    for file in files {
        let destination = native(&engine.folders.path_of(&file.rel));
        match existing(engine, &destination, file.size) {
            Ok(Some(outcome)) => finished(engine, outcome),
            Err(failure) => failures.member(file, failure, Attempt::NotPublished),
            // Exports change their length on the way: they go alone.
            Ok(None) if !matches!(source.read_size(&file.source, file.size), Ok(Some(size)) if size == file.size) => {
                run_file(engine, file, buffer)
            }
            Ok(None) => members.push((file, destination)),
        }
    }
    if members.len() < 2 {
        for (file, _) in members {
            run_file(engine, file, buffer);
        }
        return failures.finish();
    }
    let bytes: u64 = members.iter().map(|(file, _)| file.size).sum();
    // The whole packet must fit like a single download would (K7).
    if let Err(message) = ensure_local_space(&members[0].1, bytes) {
        engine.fatal(target_refuses(&io::Error::new(
            io::ErrorKind::StorageFull,
            message,
        )));
        return;
    }
    let Some(reservation) = reserve_memory(bytes, engine.stop_flag()) else {
        return;
    };
    let Some(permits) = engine.acquire() else {
        return;
    };
    let active = engine
        .stats
        .begin(&format!("{} kleine Dateien (Paket)", members.len()));
    engine.queue.notify();
    let items: Vec<BatchGet> = members
        .iter()
        .map(|(file, _)| BatchGet {
            path: file.source.clone(),
            id: file.id.clone(),
            size: file.size,
        })
        .collect();
    let meter = Meter::new(&permits, &engine.stats);
    let started = Instant::now();
    let (result, outcomes, received) = {
        let mut sink = Sink {
            engine,
            members: &members,
            current: None,
            outcomes: (0..members.len()).map(|_| None).collect(),
            received: vec![0; members.len()],
            meter: &meter,
        };
        let result = source.get_batch(&items, &mut sink);
        (result, sink.outcomes, sink.received)
    };
    permits.finish(match &result {
        Ok(()) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    for (((file, _), outcome), received) in members.into_iter().zip(outcomes).zip(received) {
        match outcome {
            Some(Member::Done(Ok(outcome))) => finished(engine, outcome),
            Some(Member::Done(Err(failure))) => {
                engine.stats.unmoved(received);
                failures.member(file, failure, Attempt::NotPublished);
            }
            Some(Member::NotSent(error)) => {
                engine.stats.unmoved(received);
                not_sent(engine, source, file, error, &mut failures, buffer);
            }
            None => {
                engine.stats.unmoved(received);
                let error = match &result {
                    Err(error) => relabeled(error, "Paket-Download"),
                    Ok(()) => io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Paket-Download lieferte diese Datei nicht",
                    ),
                };
                failures.member(file, OpError::source(error), Attempt::NotPublished);
            }
        }
    }
    failures.finish();
}

/// An item the peer did not send (nothing arrived, nothing was published).
/// When its length or time changed since the listing it goes alone with
/// what it is now; otherwise its failure is decided like any other.
fn not_sent(
    engine: &Engine<'_>,
    source: &dyn Backend,
    mut file: FileWork,
    error: io::Error,
    failures: &mut Failures<'_, '_>,
    buffer: &mut [u8],
) {
    match current(engine, source, &file) {
        Some(now) if now.size != file.size || listed_time_differs(file.mtime_ms, now.mtime_ms) => {
            engine.stats.resized(file.size, now.size);
            file.size = now.size;
            file.mtime_ms = now.mtime_ms;
            file.md5 = now.content_md5;
            if now.id.is_some() {
                file.id = now.id;
            }
            file.alone = true;
            run_file(engine, file, buffer);
        }
        _ => failures.member(file, OpError::source(error), Attempt::NotPublished),
    }
}

/// One stat of a packet item under a listing permit; `None` when it is no
/// plain file (any more) or cannot be looked at.
fn current(engine: &Engine<'_>, source: &dyn Backend, file: &FileWork) -> Option<VfsMeta> {
    let permit = engine.source_flow.acquire_meta(engine.stop_flag())?;
    let stat = source.stat(&file.source);
    permit.finish(match &stat {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    stat.ok().filter(|meta| !meta.is_dir && !meta.is_symlink)
}

/// How a packet item ended.
enum Member {
    Done(OpResult),
    /// The peer did not send it.
    NotSent(io::Error),
}

/// The part being written for the current packet item.
struct Current {
    index: usize,
    part: Option<Part>,
    check: Option<RemoteCheck>,
    failure: Option<OpError>,
    /// The peer sends another length than listed: nothing is kept, the file
    /// goes alone with its current length.
    changed: bool,
}

struct Sink<'s, 'e, 'm> {
    engine: &'s Engine<'e>,
    members: &'s [(FileWork, PathBuf)],
    current: Option<Current>,
    outcomes: Vec<Option<Member>>,
    /// Bytes of each item the progress counted.
    received: Vec<u64>,
    meter: &'s Meter<'m>,
}

impl Sink<'_, '_, '_> {
    fn member(&self, index: usize) -> io::Result<&(FileWork, PathBuf)> {
        self.members
            .get(index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Paket-Eintrag unbekannt"))
    }

    fn record(&mut self, index: usize, outcome: Member) {
        if let Some(slot) = self.outcomes.get_mut(index) {
            *slot = Some(outcome);
        }
    }

    /// A local write failed: a target that takes nothing more stops the
    /// packet (the job ends), anything else only costs this item.
    fn local_failure(&mut self, error: io::Error) -> io::Result<()> {
        let stop = ends_job_at_target(error.kind());
        let copy = io::Error::new(error.kind(), error.to_string());
        if let Some(current) = self.current.as_mut() {
            current.part = None;
            current.failure = Some(OpError::target(error));
        }
        if stop {
            if let Some(current) = self.current.take() {
                let failure = current.failure.unwrap_or_else(|| {
                    OpError::target(io::Error::new(copy.kind(), copy.to_string()))
                });
                self.record(current.index, Member::Done(Err(failure)));
            }
            return Err(copy);
        }
        Ok(())
    }
}

impl BatchSink for Sink<'_, '_, '_> {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        let (file, destination) = self.member(index)?;
        let changed = size != file.size;
        let check = Some(RemoteCheck::new(file, Some(file.size)));
        let part = if changed {
            Ok(None)
        } else {
            Part::create(destination).map(Some)
        };
        self.current = Some(Current {
            index,
            part: None,
            check,
            failure: None,
            changed,
        });
        match part {
            Ok(part) => {
                if let Some(current) = self.current.as_mut() {
                    current.part = part;
                }
                Ok(())
            }
            Err(failure) => self.local_failure(failure.error),
        }
    }

    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        let Some(current) = self
            .current
            .as_mut()
            .filter(|current| current.index == index)
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Paket-Daten ohne Beginn",
            ));
        };
        if current.failure.is_some() || current.changed {
            return Ok(());
        }
        let (file, _) = self
            .members
            .get(index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Paket-Eintrag unbekannt"))?;
        let written = current.part.as_ref().map_or(0, Part::written);
        if let Some(check) = current.check.as_mut() {
            if let Err(failure) = check.update(file, written, bytes) {
                current.failure = Some(failure);
                current.part = None;
                return Ok(());
            }
        }
        if let Some(part) = current.part.as_mut() {
            if let Err(failure) = part.write(bytes) {
                return self.local_failure(failure.error);
            }
        }
        self.meter.add(bytes.len() as u64);
        if let Some(received) = self.received.get_mut(index) {
            *received += bytes.len() as u64;
        }
        Ok(())
    }

    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        let current = match self.current.take() {
            Some(current) if current.index == index => current,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Paket-Ende ohne Beginn",
                ))
            }
        };
        let (file, destination) = self.member(index)?;
        let outcome = match (result, current.failure) {
            (_, Some(failure)) => Member::Done(Err(failure)),
            _ if current.changed => Member::NotSent(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: Quelle wurde seit der Auflistung geändert", file.source),
            )),
            (Err(error), None) => Member::Done(Err(OpError::source(error))),
            (Ok(()), None) => Member::Done(match (current.part, current.check) {
                (Some(part), Some(check)) => check
                    .finish_streamed(file, part.written())
                    .and_then(|()| publish_local(self.engine, part, destination, file)),
                _ => Err(OpError::target(io::Error::other(
                    "Paket-Eintrag ohne Teildatei",
                ))),
            }),
        };
        self.record(index, outcome);
        Ok(())
    }

    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.index == index)
        {
            self.current = None;
        }
        self.record(index, Member::NotSent(error));
        Ok(())
    }
}
