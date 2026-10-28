//! Packets inside one account (G3-13, 2026-10-28): the server copies itself,
//! and where it takes several copies at once (`vfs::server_copy_batch`: SFTP
//! runs up to 64 `copy-data` chains on one channel) one call asks for the
//! server copies of a whole packet into private stages instead of one round
//! trip per file. Each member is then handled exactly like a single file in
//! `remote_copy::server_copy`: a copy of another length is a changed source,
//! the source must still be the listed file before its copy is published
//! (K21), publication is the create-only `publish::publish` (numbered when
//! the name is taken), and a member the server did not copy goes alone.
//! A server that copies none of a packet ends packets for the job, so a
//! provider without the batch costs one empty call.
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::batch::{Attempt, Failures};
use super::ops::{At, Meter, OpError};
use super::publish;
use super::queue::{FileWork, Work};
use super::source::unchanged_since_listing;
use super::worker::finished;
use super::Engine;
use crate::vfs::{Backend, BatchLimits};
use std::io;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Most server copies one call asks for; a stop is seen between calls.
pub(super) const SERVER_BATCH: usize = 64;

/// Packets of server copies: the file count of one call. The bytes never
/// pass through this process; the bound only keeps one packet's copy time
/// near the sizer's target.
pub(super) const SERVER_PACKET: BatchLimits = BatchLimits {
    max_files: SERVER_BATCH,
    max_bytes: 64 * 1024 * 1024,
};

/// Whether a packet inside one account was answered with no copy at all:
/// later files of the job go alone (`batch::members`).
pub(super) fn refused(engine: &Engine<'_>) -> bool {
    engine.view.source.same_namespace(engine.view.target)
        && engine.server_packets_refused.load(Ordering::Acquire)
}

/// One packet of `files`, copied by the server of `target` (the account of
/// `source`) and published one by one.
pub(super) fn copy(
    engine: &Engine<'_>,
    source: &dyn Backend,
    target: &dyn Backend,
    mut files: Vec<FileWork>,
) {
    let Some(permits) = engine.acquire() else {
        return;
    };
    let active = engine
        .stats
        .begin(&format!("{} Serverkopien (Paket)", files.len()));
    engine.queue.notify();
    let meter = Meter::new(&permits, &engine.stats);
    let started = Instant::now();
    let mut failures = Failures::new(engine);
    let mut alone = Vec::new();
    let mut outcome = OpOutcome::Done;
    let mut copied_bytes = 0u64;
    while !files.is_empty() {
        if engine.stopped() {
            break;
        }
        let chunk: Vec<FileWork> = files.drain(..files.len().min(SERVER_BATCH)).collect();
        let staged: Vec<(String, String)> = chunk
            .iter()
            .map(|file| {
                let path = engine.folders.path_of(&file.rel);
                let stage = publish::stage_name(&path);
                (path, stage)
            })
            .collect();
        let items: Vec<(String, String, u64)> = chunk
            .iter()
            .zip(&staged)
            .map(|(file, (_, stage))| (file.source.clone(), stage.clone(), file.size))
            .collect();
        // The target server copies while this thread waits for its answers.
        let answers = meter.timed_target(|| crate::vfs::server_copy_batch(target, &items));
        let none_copied =
            answers.len() == items.len() && answers.iter().all(|answer| matches!(answer, Ok(None)));
        if none_copied {
            // Nothing of the attempt exists: no batch here (or no longer).
            engine.server_packets_refused.store(true, Ordering::Release);
            alone.extend(chunk);
            alone.append(&mut files);
            break;
        }
        let mut answers = answers.into_iter();
        for (file, (path, stage)) in chunk.into_iter().zip(staged) {
            let answer = answers.next().unwrap_or_else(|| {
                Err(io::Error::other("Serverkopie im Paket blieb ohne Antwort"))
            });
            let member = Member {
                engine,
                source,
                target,
                meter: &meter,
            };
            match member.settle(file, answer, &stage, path) {
                Settled::Done(copied) => copied_bytes = copied_bytes.saturating_add(copied),
                Settled::Alone(file) => alone.push(file),
                Settled::Failed(file, failure, attempt) => {
                    if outcome == OpOutcome::Done && failure.at != At::Unknown {
                        outcome = classify_error(&failure.error);
                    }
                    failures.member(file, failure, attempt);
                }
                Settled::Dropped => {}
            }
        }
    }
    permits.finish_with(outcome, meter.accounts());
    drop(active);
    super::lock(&engine.sizer).record(copied_bytes, started.elapsed().as_millis() as u64);
    // Members the server did not copy stream alone, on whichever worker is
    // free (never in a packet again).
    if !engine.stopped() {
        for mut file in alone.into_iter().rev() {
            file.alone = true;
            engine.queue.push_front(Work::File(file));
        }
    }
    failures.finish();
}

/// What became of one member.
enum Settled {
    /// Published (or left alone as the same file on resume), these bytes.
    Done(u64),
    /// Not copied by the server and nothing of it is left: goes alone.
    Alone(FileWork),
    /// Failed; decided like a member of any packet.
    Failed(FileWork, OpError, Attempt),
    /// The job stops: its stage is gone, nothing more happens.
    Dropped,
}

struct Member<'m, 'e> {
    engine: &'m Engine<'e>,
    source: &'m dyn Backend,
    target: &'m dyn Backend,
    meter: &'m Meter<'m>,
}

impl Member<'_, '_> {
    /// `remote_copy::server_copy` for one answer of the packet.
    fn settle(
        &self,
        file: FileWork,
        answer: io::Result<Option<u64>>,
        stage: &str,
        path: String,
    ) -> Settled {
        let engine = self.engine;
        let copied = match answer {
            Ok(None) => return Settled::Alone(file),
            // The stage name was taken and left alone: the single path
            // tries other names.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Settled::Alone(file);
            }
            Err(error) if crate::vfs::congestion_of(&error).is_some() => {
                publish::discard(engine, self.target, stage);
                return Settled::Failed(file, OpError::target(error), Attempt::NotPublished);
            }
            // A server copy is only a shortcut: the file streams, which
            // attributes a real problem to the side that has it.
            Err(_) => {
                publish::discard(engine, self.target, stage);
                return if engine.stopped() {
                    Settled::Dropped
                } else {
                    Settled::Alone(file)
                };
            }
            Ok(Some(copied)) => copied,
        };
        if copied != file.size {
            publish::discard(engine, self.target, stage);
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: Quelle wurde während der Übertragung geändert",
                    file.source
                ),
            );
            return Settled::Failed(file, OpError::source(error), Attempt::NotPublished);
        }
        if engine.stopped() {
            publish::discard(engine, self.target, stage);
            return Settled::Dropped;
        }
        // The server copied whatever the source was by then: published only
        // while that is still the listed file (K21).
        let unchanged = self
            .meter
            .timed_source(|| unchanged_since_listing(self.source, &file));
        if let Err(changed) = unchanged {
            publish::discard(engine, self.target, stage);
            return Settled::Failed(file, changed, Attempt::NotPublished);
        }
        match publish::publish(engine, self.target, &file, stage, path) {
            Ok(outcome) => {
                self.meter.add(copied);
                finished(engine, outcome);
                Settled::Done(copied)
            }
            Err(failure) => {
                let attempt = if failure.at == At::Unknown {
                    Attempt::MaybePublished
                } else {
                    Attempt::NotPublished
                };
                Settled::Failed(file, failure, attempt)
            }
        }
    }
}
