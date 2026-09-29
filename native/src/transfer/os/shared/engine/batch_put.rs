//! Upload packets: the files of a packet stream one after another from disk
//! into `put_batch` (nothing is buffered first, K7). The chunk that completes
//! a file goes out only when the file ended there and did not change while
//! it was read; otherwise the packet stream fails at that file, so the peer
//! discards it and publishes nothing changed. What the stream delivered
//! decides which members of a failed packet may go again.
use super::super::engine_names::{base_name, parent_rel};
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::super::memory::reserve_memory;
use super::batch::{Attempt, Failures};
use super::ops::{relabeled, At, Meter, OpError, Outcome};
use super::queue::FileWork;
use super::source::LocalSource;
use super::worker::{finished, run_file};
use super::Engine;
use crate::vfs::{Backend, BatchPut, BatchPutOutcome};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub(super) fn upload(
    engine: &Engine<'_>,
    target: &dyn Backend,
    files: Vec<FileWork>,
    buffer: &mut [u8],
) {
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
    let (result, delivery) = {
        let mut data = PacketData::new(
            opened.iter_mut().map(|(_, source)| source).collect(),
            &meter,
            engine.stop_flag(),
        );
        let result = target.put_batch(&entries, &mut data);
        (result, data.delivery())
    };
    permits.finish(match &result {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    let files: Vec<FileWork> = opened.into_iter().map(|(file, _)| file).collect();
    let outcomes = match result {
        Ok(outcomes) => outcomes,
        // Refused before anything was sent: each file goes alone.
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            engine.stats.unmoved(delivery.served.iter().sum());
            for file in files {
                run_file(engine, file, buffer);
            }
            return;
        }
        Err(error) => return packet_lost(engine, files, &delivery, &error),
    };
    let mut failures = Failures::new(engine);
    let mut outcomes = outcomes.into_iter();
    for (index, file) in files.into_iter().enumerate() {
        match outcomes.next() {
            Some(BatchPutOutcome::Published(path)) => {
                if parent_rel(&file.rel).is_none() {
                    engine.folders.record_alias(&file.rel, base_name(&path));
                }
                finished(engine, Outcome::Done);
            }
            Some(BatchPutOutcome::Failed(error)) => {
                engine.stats.unmoved(delivery.served[index]);
                let (failure, attempt) = delivery.judge(index, OpError::target(error));
                failures.member(file, failure, attempt);
            }
            None => {
                engine.stats.unmoved(delivery.served[index]);
                let missing =
                    io::Error::other("Paket-Upload lieferte kein Ergebnis für diese Datei");
                let (failure, attempt) = delivery.judge(index, OpError::at(At::Unknown, missing));
                failures.member(file, failure, attempt);
            }
        }
    }
    failures.finish();
}

/// The packet's answer was lost (`Err`): what was not completely sent goes
/// again, everything else has an unknown result.
fn packet_lost(engine: &Engine<'_>, files: Vec<FileWork>, delivery: &Delivery, error: &io::Error) {
    engine.stats.unmoved(delivery.served.iter().sum());
    if engine.stopped() {
        return;
    }
    let mut failures = Failures::new(engine);
    for (index, file) in files.into_iter().enumerate() {
        let lost = OpError::at(At::Unknown, relabeled(error, "Paket-Upload"));
        let (failure, attempt) = delivery.judge(index, lost);
        failures.member(file, failure, attempt);
    }
    failures.finish();
}

/// What the packet stream handed to the peer.
struct Delivery {
    lengths: Vec<u64>,
    /// Bytes of each entry the stream delivered (and the progress counted).
    served: Vec<u64>,
    /// The entry whose read or final check failed; the stream ended there.
    failed: Option<(usize, io::ErrorKind, String)>,
}

impl Delivery {
    /// How a failed entry is treated. Its own read or check failure is its
    /// reason (discarded by the peer); an entry whose bytes did not all go
    /// out, or that came after the failed one, was never tried; any other
    /// entry may have been published.
    fn judge(&self, index: usize, failure: OpError) -> (OpError, Attempt) {
        match &self.failed {
            Some((at, kind, message)) if *at == index => (
                OpError::source(io::Error::new(*kind, message.clone())),
                Attempt::NotPublished,
            ),
            Some((at, ..)) if index > *at => (failure, Attempt::Untried),
            _ if self.served[index] < self.lengths[index] => (failure, Attempt::Untried),
            _ => (failure, Attempt::MaybePublished),
        }
    }
}

/// The files of a packet back to back, read straight from disk.
struct PacketData<'s, 'm> {
    sources: Vec<&'s mut LocalSource>,
    index: usize,
    sent: u64,
    served: Vec<u64>,
    failed: Option<(usize, io::ErrorKind, String)>,
    meter: &'s Meter<'m>,
    stop: &'s AtomicBool,
}

impl<'s, 'm> PacketData<'s, 'm> {
    fn new(sources: Vec<&'s mut LocalSource>, meter: &'s Meter<'m>, stop: &'s AtomicBool) -> Self {
        let served = vec![0; sources.len()];
        Self {
            sources,
            index: 0,
            sent: 0,
            served,
            failed: None,
            meter,
            stop,
        }
    }

    fn delivery(self) -> Delivery {
        Delivery {
            lengths: self.sources.iter().map(|source| source.length()).collect(),
            served: self.served,
            failed: self.failed,
        }
    }
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
            if let Some((_, kind, message)) = &self.failed {
                return Err(io::Error::new(*kind, message.clone()));
            }
            let Some(source) = self.sources.get_mut(self.index) else {
                return Ok(0);
            };
            if self.sent >= source.length() {
                self.index += 1;
                self.sent = 0;
                continue;
            }
            if buffer.is_empty() {
                return Ok(0);
            }
            match source.read_entry(buffer, self.sent) {
                Ok(read) => {
                    self.sent += read as u64;
                    self.served[self.index] += read as u64;
                    self.meter.add(read as u64);
                    return Ok(read);
                }
                Err(failure) => {
                    let (kind, message) = (failure.error.kind(), failure.error.to_string());
                    self.failed = Some((self.index, kind, message.clone()));
                    return Err(io::Error::new(kind, message));
                }
            }
        }
    }
}
