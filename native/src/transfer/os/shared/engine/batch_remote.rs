//! Remote → remote packets (G3-13, 2026-10-28): between two connections that
//! both move packets (Share hosts, the SSH agent, SFTP, the worker), the
//! small files of a packet come from the source in one `get_batch` round
//! trip into the engine's reserved memory and go to the target as one
//! `put_batch`, instead of one read stream, one stage and one publication
//! round trip per file. Inside one account the server copies itself
//! (`remote_copy::server_copy`), so no packet is formed there.
//!
//! The rules of both packet kinds hold: a member reaches the target only
//! when the source delivered exactly its listed bytes (and its listed MD5);
//! its intent is durable before the target can publish it; the target
//! publishes without replacing (numbered name when taken); a member whose
//! result is unknown is reported and never sent again; a member that was
//! certainly not published goes alone again (once for a transient failure);
//! a source that changed since the listing goes alone with what it is now.
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::super::memory::reserve_memory;
use super::batch::{Attempt, Failures};
use super::batch_put::{host_evidence, packet_lost, settle, without_evidence, Delivery};
use super::file_evidence::HostEvidence;
use super::file_recovery::PacketIntent;
use super::ops::{nanos, relabeled, Meter, OpError};
use super::queue::FileWork;
use super::view::JobView;
use super::worker::run_file;
use super::Engine;
use crate::vfs::{Backend, BatchGet, BatchLimits, BatchPut, EvidencePutOutcome, FileEvidence};
use gather::{Gather, Slot};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Packet limits between two connections: what both sides take, so one
/// packet is one `get_batch` and one `put_batch`. `None` inside one account
/// (the server copies) and where either side moves no packets.
pub(super) fn limits(
    view: &JobView<'_>,
    source: &dyn Backend,
    target: &dyn Backend,
    first: &str,
) -> Option<BatchLimits> {
    if view.source.same_namespace(view.target) {
        return None;
    }
    let from = super::source_batch_limits(source, first)?;
    let to = target.batch_limits(view.target_dir)?;
    Some(packet_limits(from, to))
}

/// The smaller bound of each kind.
pub(super) fn packet_limits(from: BatchLimits, to: BatchLimits) -> BatchLimits {
    BatchLimits {
        max_files: from.max_files.min(to.max_files),
        max_bytes: from.max_bytes.min(to.max_bytes),
    }
}

/// Copies one packet of `files` from `source` to `target`.
pub(super) fn copy(
    engine: &Engine<'_>,
    source: &dyn Backend,
    target: &dyn Backend,
    files: Vec<FileWork>,
    buffer: &mut [u8],
) {
    // Exports change their length on the way: they go alone.
    let mut members = Vec::with_capacity(files.len());
    for file in files {
        match source.read_size(&file.source, file.size) {
            Ok(Some(size)) if size == file.size => members.push(file),
            _ => run_file(engine, file, buffer),
        }
    }
    if members.len() < 2 {
        for file in members {
            run_file(engine, file, buffer);
        }
        return;
    }
    // The whole packet is held in memory between the two sides (K7).
    let bytes: u64 = members.iter().map(|file| file.size).sum();
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
        .begin(&format!("{} kleine Dateien (Paket)", members.len()));
    engine.queue.notify();
    let meter = Meter::new(&permits, &engine.stats);
    meter.add_wait(waited);
    let started = Instant::now();
    let items: Vec<BatchGet> = members
        .iter()
        .map(|file| BatchGet {
            path: file.source.clone(),
            id: file.id.clone(),
            size: file.size,
        })
        .collect();
    let mut gather = Gather::new(&members, &meter, engine.stop_flag());
    // The whole read, hashing included, is the source's time.
    let fetched = meter.timed_source(|| source.get_batch(&items, &mut gather));
    let gathered = gather.slots;
    let refused = matches!(&fetched, Err(error) if error.kind() == io::ErrorKind::Unsupported);
    if refused && gathered.iter().all(Option::is_none) {
        let outcome = match &fetched {
            Ok(()) => OpOutcome::Done,
            Err(error) => classify_error(error),
        };
        permits.finish_with(outcome, meter.accounts());
        drop(active);
        drop(reservation);
        for file in members {
            run_file(engine, file, buffer);
        }
        return;
    }
    let mut failures = Failures::new(engine);
    let mut ready = Vec::new();
    let mut alone = Vec::new();
    for (file, slot) in members.into_iter().zip(gathered) {
        match slot {
            Some(Slot::Ready { data, evidence }) => ready.push((file, data, evidence)),
            Some(Slot::Failed(failure)) => failures.member(file, failure, Attempt::NotPublished),
            Some(Slot::NotSent(error)) => alone.push((file, error)),
            None => {
                let error = match &fetched {
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
    let sent = send(engine, target, ready, &meter);
    let accounts = meter.accounts();
    permits.finish_with(
        match (&fetched, &sent) {
            (
                _,
                Sent::Packet {
                    result: Err(error), ..
                },
            ) => classify_error(error),
            (Err(error), _) => classify_error(error),
            _ => OpOutcome::Done,
        },
        accounts,
    );
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    // Everything that may wait or start another file runs without the
    // packet's permits.
    settle_sent(engine, sent, buffer);
    // Items the source did not send: alone with what they are now.
    for (file, error) in alone {
        super::batch_get::not_sent(engine, source, file, error, &mut failures, buffer);
    }
    failures.finish();
}

/// What became of the ready members.
enum Sent {
    /// Nothing was sent; these go alone (fewer than two members).
    Alone(Vec<FileWork>),
    /// The intents could not be written: nothing was sent, the job ends.
    Unprepared,
    /// The packet went out; `result` is the target's answer.
    Packet {
        files: Vec<FileWork>,
        delivery: Delivery,
        result: io::Result<Vec<EvidencePutOutcome>>,
        prepare_failure: Option<String>,
    },
}

/// Sends the ready members as one `put_batch` under the packet's permits.
fn send(
    engine: &Engine<'_>,
    target: &dyn Backend,
    ready: Vec<(FileWork, Vec<u8>, Option<FileEvidence>)>,
    meter: &Meter<'_>,
) -> Sent {
    if ready.len() < 2 {
        return Sent::Alone(ready.into_iter().map(|(file, _, _)| file).collect());
    }
    let mut files = Vec::with_capacity(ready.len());
    let mut parts = Vec::with_capacity(ready.len());
    let mut sources = Vec::with_capacity(ready.len());
    for (file, data, evidence) in ready {
        // Evidence of another path than the member's is no evidence of it.
        let known = evidence
            .filter(|evidence| evidence.path == file.source)
            .map(host_evidence);
        files.push(file);
        parts.push(data);
        sources.push(known);
    }
    let entries: Vec<BatchPut> = files
        .iter()
        .zip(&parts)
        .map(|(file, data)| BatchPut {
            path: engine.folders.path_of(&file.rel),
            size: data.len() as u64,
        })
        .collect();
    let evidenced = crate::vfs::transfer_evidence(target, engine.folders.root());
    if !evidenced {
        // One record for the packet before anything can be published.
        let unknown = vec![None; files.len()];
        if let Err(error) = prepare(engine, &files, &sources, 0, &unknown) {
            for file in &files {
                let _ = super::file_recovery::unpublished(engine, file);
            }
            engine.fatal(format!("Paketpublikation vorbereiten: {error}"));
            return Sent::Unprepared;
        }
    }
    let mut prepare_failure: Option<String> = None;
    let started = Instant::now();
    let (result, delivery) = {
        let mut data = MemoryData::new(&parts, meter, engine.stop_flag());
        let result = if evidenced {
            let mut prepare_part = |offset: usize, existed: &[Option<bool>]| {
                prepare(engine, &files, &sources, offset, existed).map_err(|error| {
                    prepare_failure = Some(error.to_string());
                    error
                })
            };
            crate::vfs::put_batch_evidence(target, &entries, &mut data, &mut prepare_part)
        } else {
            target.put_batch(&entries, &mut data).map(without_evidence)
        };
        (result, data.delivery(&parts))
    };
    meter.add_target_ns(nanos(started.elapsed()));
    Sent::Packet {
        files,
        delivery,
        result,
        prepare_failure,
    }
}

/// Settles the target's answer (after the permits went back): published
/// members get their receipts, failed ones are decided by what the target
/// received, a refused packet sends every member alone.
fn settle_sent(engine: &Engine<'_>, sent: Sent, buffer: &mut [u8]) {
    let (files, delivery, result, prepare_failure) = match sent {
        Sent::Alone(files) => {
            for file in files {
                run_file(engine, file, buffer);
            }
            return;
        }
        Sent::Unprepared => return,
        Sent::Packet {
            files,
            delivery,
            result,
            prepare_failure,
        } => (files, delivery, result, prepare_failure),
    };
    match result {
        Ok(outcomes) => settle(engine, files, outcomes, &delivery),
        // Refused before anything was read: each member goes alone.
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

/// The intents of the members `offset..` as one record, with the source as
/// the source's packet described it and the destinations' presence as the
/// target reported it (`file_recovery::prepare_batch_known`).
fn prepare(
    engine: &Engine<'_>,
    files: &[FileWork],
    sources: &[Option<HostEvidence>],
    offset: usize,
    existed: &[Option<bool>],
) -> io::Result<()> {
    let end = offset.saturating_add(existed.len());
    let (Some(members), Some(known)) = (files.get(offset..end), sources.get(offset..end)) else {
        return Err(io::Error::other("Paketannahme nennt unbekannte Einträge"));
    };
    let intents: Vec<PacketIntent<'_>> = members
        .iter()
        .zip(known)
        .zip(existed)
        .map(|((file, source), existed)| PacketIntent {
            file,
            source: source.as_ref(),
            existed_before: *existed,
        })
        .collect();
    super::file_recovery::prepare_batch_known(engine, &intents)
}

/// The members' bytes back to back from memory.
struct MemoryData<'s, 'm> {
    parts: &'s [Vec<u8>],
    index: usize,
    sent: usize,
    served: Vec<u64>,
    meter: &'s Meter<'m>,
    stop: &'s AtomicBool,
}

impl<'s, 'm> MemoryData<'s, 'm> {
    fn new(parts: &'s [Vec<u8>], meter: &'s Meter<'m>, stop: &'s AtomicBool) -> Self {
        Self {
            parts,
            index: 0,
            sent: 0,
            served: vec![0; parts.len()],
            meter,
            stop,
        }
    }

    fn delivery(self, parts: &[Vec<u8>]) -> Delivery {
        Delivery {
            lengths: parts.iter().map(|part| part.len() as u64).collect(),
            served: self.served,
            // Memory cannot fail to read: no member ends the stream early.
            failed: None,
        }
    }
}

impl Read for MemoryData<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    super::super::cancel::CANCELED_ERROR,
                ));
            }
            let Some(part) = self.parts.get(self.index) else {
                return Ok(0);
            };
            if self.sent >= part.len() {
                self.index += 1;
                self.sent = 0;
                continue;
            }
            if buffer.is_empty() {
                return Ok(0);
            }
            let count = buffer.len().min(part.len() - self.sent);
            buffer[..count].copy_from_slice(&part[self.sent..self.sent + count]);
            self.sent += count;
            self.served[self.index] += count as u64;
            self.meter.add(count as u64);
            return Ok(count);
        }
    }
}

#[path = "batch_remote_gather.rs"]
mod gather;

#[cfg(test)]
#[path = "batch_remote_tests.rs"]
mod tests;
