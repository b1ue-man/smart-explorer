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
use super::file_evidence::HostEvidence;
use super::file_recovery::PacketIntent;
use super::ops::{nanos, relabeled, At, Meter, OpError, Outcome};
use super::queue::FileWork;
use super::source::LocalSource;
use super::worker::{finished, run_file};
use super::Engine;
use crate::vfs::{Backend, BatchPut, BatchPutOutcome, EvidencePutOutcome, FileEvidence};
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
            // Recheck the actual admitted reader after the queue probe: media
            // replacement must not leave an optical permit in the open packet.
            Ok(source) if source.is_optical() => {
                drop(source);
                run_file(engine, file, buffer);
            }
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
    let (files, mut sources): (Vec<FileWork>, Vec<LocalSource>) = opened.into_iter().unzip();
    let bytes: u64 = sources.iter().map(LocalSource::length).sum();
    let memory_asked = Instant::now();
    let Some(reservation) = reserve_memory(bytes, engine.stop_flag()) else {
        return;
    };
    let memory_wait = nanos(memory_asked.elapsed());
    engine.stats.book_memory_wait(memory_wait);
    let permit_asked = Instant::now();
    let Some(permits) = engine.acquire() else {
        return;
    };
    let waited = memory_wait.saturating_add(nanos(permit_asked.elapsed()));
    let active = engine
        .stats
        .begin(&format!("{} kleine Dateien (Paket)", files.len()));
    engine.queue.notify();
    let entries: Vec<BatchPut> = files
        .iter()
        .zip(&sources)
        .map(|(file, source)| BatchPut {
            path: engine.folders.path_of(&file.rel),
            size: source.length(),
            // The source's time, where the peer sets it (`publish_time.rs`).
            mtime_ms: (file.mtime_ms > 0).then_some(file.mtime_ms),
        })
        .collect();
    let meter = Meter::new(&permits, &engine.stats);
    meter.add_wait(waited);
    // A peer with transfer evidence names the existing destinations before
    // its commit and returns each published file's evidence: the intents
    // are written in that moment (`prepare`), still before anything can be
    // published, and no member costs a request of its own.
    let evidenced = crate::vfs::transfer_evidence(target, engine.folders.root());
    // Packet protocols publish before returning their object IDs. Persist
    // the uncertainty first; a lost answer can never authorize a resend.
    // One record for the whole packet; without the peer's report each
    // member's previous presence is looked up as before.
    if !evidenced {
        let unknown = vec![None; files.len()];
        if let Err(error) = prepare_part(engine, &files, 0, &unknown) {
            for prepared in &files {
                let _ = super::file_recovery::unpublished(engine, prepared);
            }
            engine.fatal(format!("Paketpublikation vorbereiten: {error}"));
            return;
        }
    }
    let mut prepare_failure: Option<String> = None;
    let started = Instant::now();
    let read_before = meter.accounts().source_ns;
    let (result, delivery) = {
        let mut data = PacketData::new(sources.iter_mut().collect(), &meter, engine.stop_flag());
        let result = if evidenced {
            let mut prepare = |offset: usize, existed: &[Option<bool>]| {
                prepare_part(engine, &files, offset, existed).inspect_err(|error| {
                    prepare_failure = Some(error.to_string());
                })
            };
            crate::vfs::put_batch_evidence(target, &entries, &mut data, &mut prepare)
        } else {
            target.put_batch(&entries, &mut data).map(without_evidence)
        };
        (result, data.delivery())
    };
    drop(sources);
    // The packet's reads are booked to the source as they happen; the rest
    // of the call is the wire and the peer: the target.
    let read = meter.accounts().source_ns.saturating_sub(read_before);
    meter.add_target_ns(nanos(started.elapsed()).saturating_sub(read));
    let accounts = meter.accounts();
    permits.finish_with(
        match &result {
            Ok(_) => OpOutcome::Done,
            Err(error) => classify_error(error),
        },
        accounts,
    );
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    match result {
        Ok(outcomes) => settle(engine, files, outcomes, &delivery),
        // Refused before anything was sent: each file goes alone.
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            engine.stats.unmoved(delivery.served.iter().sum());
            for file in files {
                if let Err(error) = super::file_recovery::unpublished(engine, &file) {
                    engine.fatal(format!(
                        "Unveröffentlichten Paketbeleg zurücknehmen: {error}"
                    ));
                    return;
                }
                run_file(engine, file, buffer);
            }
        }
        Err(error) => packet_lost(engine, files, &delivery, &error),
    }
    // The member intents could not be written: nothing of that part was
    // published, and the job ends as when a single intent fails.
    if let Some(error) = prepare_failure {
        engine.fatal(format!("Paketpublikation vorbereiten: {error}"));
    }
}

/// The intents of the members `offset..` with the destinations' presence
/// the peer reported, as one record (`file_recovery::prepare_batch_known`).
fn prepare_part(
    engine: &Engine<'_>,
    files: &[FileWork],
    offset: usize,
    existed: &[Option<bool>],
) -> io::Result<()> {
    let members = files
        .get(offset..)
        .and_then(|rest| rest.get(..existed.len()))
        .ok_or_else(|| io::Error::other("Paketannahme nennt unbekannte Einträge"))?;
    let intents: Vec<PacketIntent<'_>> = members
        .iter()
        .zip(existed)
        .map(|(file, existed)| PacketIntent {
            file,
            source: None,
            existed_before: *existed,
            stage: None,
            destination: None,
        })
        .collect();
    super::file_recovery::prepare_batch_known(engine, &intents)
}

pub(super) fn without_evidence(outcomes: Vec<BatchPutOutcome>) -> Vec<EvidencePutOutcome> {
    outcomes
        .into_iter()
        .map(|outcome| match outcome {
            BatchPutOutcome::Published(path) => EvidencePutOutcome::Published {
                path,
                evidence: None,
            },
            BatchPutOutcome::Failed(error) => EvidencePutOutcome::Failed(error),
        })
        .collect()
}

/// The packet's evidence as the recovery records it.
pub(super) fn host_evidence(evidence: FileEvidence) -> HostEvidence {
    HostEvidence {
        path: evidence.path,
        object: evidence.object,
        id: evidence.id,
        size: evidence.size,
        mtime_ms: evidence.mtime_ms,
    }
}

/// Completes the members the peer answered for: a published member's
/// receipt from the peer's evidence (or, without it, by asking), a failed
/// one decided by what the stream delivered.
pub(super) fn settle(
    engine: &Engine<'_>,
    files: Vec<FileWork>,
    outcomes: Vec<EvidencePutOutcome>,
    delivery: &Delivery,
) {
    let mut failures = Failures::new(engine);
    let mut outcomes = outcomes.into_iter();
    for (index, file) in files.into_iter().enumerate() {
        match outcomes.next() {
            Some(EvidencePutOutcome::Published { path, evidence }) => {
                let acknowledged = match evidence {
                    Some(evidence) => super::file_recovery::after_known(
                        engine,
                        &file,
                        &path,
                        Some(&host_evidence(evidence)),
                    ),
                    None => super::file_recovery::after(engine, &file, &path),
                };
                if let Err(error) = acknowledged {
                    failures.member(
                        file,
                        OpError::at(At::Unknown, error),
                        Attempt::MaybePublished,
                    );
                    continue;
                }
                if parent_rel(&file.rel).is_none() {
                    engine.folders.record_alias(&file.rel, base_name(&path));
                }
                finished(engine, Outcome::Done);
            }
            Some(EvidencePutOutcome::Failed(error)) => {
                engine.stats.unmoved(delivery.served[index]);
                let (failure, mut attempt) = delivery.judge(index, OpError::target(error));
                // The peer says it published nothing for this file: it may
                // go again once, like a file whose bytes were refused.
                if attempt == Attempt::MaybePublished && crate::vfs::unpublished_of(&failure.error)
                {
                    attempt = Attempt::NotPublished;
                }
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
pub(super) fn packet_lost(
    engine: &Engine<'_>,
    files: Vec<FileWork>,
    delivery: &Delivery,
    error: &io::Error,
) {
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
pub(super) struct Delivery {
    pub(super) lengths: Vec<u64>,
    /// Bytes of each entry the stream delivered (and the progress counted).
    pub(super) served: Vec<u64>,
    /// The entry whose read or final check failed; the stream ended there.
    pub(super) failed: Option<(usize, io::ErrorKind, String)>,
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
            match self
                .meter
                .timed_source(|| source.read_entry(buffer, self.sent))
            {
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

/// A single file as a packet of one (Z-P4).
#[path = "batch_put_one.rs"]
pub(super) mod one;

#[cfg(test)]
#[path = "batch_put_speed_tests.rs"]
pub(super) mod speed_tests;
