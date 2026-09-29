//! Download packets: many small files in one `get_batch` round trip, written
//! straight into their `.part` files and published one by one under the
//! conflict policy as each completes. Local publication is exact, so a
//! failed packet leaves nothing ambiguous: unfinished files go alone again.
use super::super::memory::reserve_memory;
use super::super::walk_listers::native;
use super::batch::member_failed;
use super::download::{existing, publish_local, Part};
use super::ops::{Meter, OpError, OpResult};
use super::queue::FileWork;
use super::source::RemoteCheck;
use super::worker::{finished, run_file};
use super::Engine;
use crate::vfs::{Backend, BatchGet, BatchSink};
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
    for file in files {
        let destination = native(&engine.folders.path_of(&file.rel));
        match existing(engine, &destination, file.size) {
            Ok(Some(outcome)) => finished(engine, outcome),
            Err(failure) => member_failed(engine, file, failure),
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
        return;
    }
    let bytes: u64 = members.iter().map(|(file, _)| file.size).sum();
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
    let (result, outcomes) = {
        let mut sink = Sink {
            engine,
            members: &members,
            current: None,
            outcomes: (0..members.len()).map(|_| None).collect(),
            meter: &meter,
        };
        let result = source.get_batch(&items, &mut sink);
        (result, sink.outcomes)
    };
    let moved = meter.moved();
    permits.finish(match &result {
        Ok(()) => super::super::flow_control::OpOutcome::Done,
        Err(error) => super::super::flow::classify_error(error),
    });
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    let mut unfinished_bytes = 0u64;
    for ((file, _), outcome) in members.into_iter().zip(outcomes) {
        match outcome {
            Some(Ok(outcome)) => finished(engine, outcome),
            Some(Err(failure)) => {
                unfinished_bytes += file.size;
                member_failed(engine, file, failure);
            }
            None => {
                unfinished_bytes += file.size;
                let error = match &result {
                    Err(error) => io::Error::new(error.kind(), format!("Paket-Download: {error}")),
                    Ok(()) => io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Paket-Download lieferte diese Datei nicht",
                    ),
                };
                member_failed(engine, file, OpError::source(error));
            }
        }
    }
    engine.stats.unmoved(unfinished_bytes.min(moved));
}

/// The part being written for the current packet item.
struct Current {
    index: usize,
    part: Option<Part>,
    check: Option<RemoteCheck>,
    failure: Option<OpError>,
}

struct Sink<'s, 'e, 'm> {
    engine: &'s Engine<'e>,
    members: &'s [(FileWork, PathBuf)],
    current: Option<Current>,
    outcomes: Vec<Option<OpResult>>,
    meter: &'s Meter<'m>,
}

impl Sink<'_, '_, '_> {
    fn member(&self, index: usize) -> io::Result<&(FileWork, PathBuf)> {
        self.members
            .get(index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Paket-Eintrag unbekannt"))
    }

    fn record(&mut self, index: usize, outcome: OpResult) {
        if let Some(slot) = self.outcomes.get_mut(index) {
            *slot = Some(outcome);
        }
    }
}

impl BatchSink for Sink<'_, '_, '_> {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        let (file, destination) = self.member(index)?;
        let mut current = Current {
            index,
            part: None,
            check: Some(RemoteCheck::new(file, Some(file.size))),
            failure: None,
        };
        if size != file.size {
            current.failure = Some(OpError::source(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: Quelle wurde während der Übertragung geändert",
                    file.source
                ),
            )));
        } else {
            match Part::create(destination) {
                Ok(part) => current.part = Some(part),
                Err(failure) => return Err(failure.error),
            }
        }
        self.current = Some(current);
        Ok(())
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
        if current.failure.is_some() {
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
            // A local write failure (full disk) stops the packet.
            part.write(bytes).map_err(|failure| failure.error)?;
        }
        self.meter.add(bytes.len() as u64);
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
            (_, Some(failure)) => Err(failure),
            (Err(error), None) => Err(OpError::source(error)),
            (Ok(()), None) => match (current.part, current.check) {
                (Some(part), Some(check)) => check
                    .finish_streamed(file, part.written())
                    .and_then(|()| publish_local(self.engine, part, destination, file)),
                _ => Err(OpError::target(io::Error::other(
                    "Paket-Eintrag ohne Teildatei",
                ))),
            },
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
        self.record(index, Err(OpError::source(error)));
        Ok(())
    }
}
