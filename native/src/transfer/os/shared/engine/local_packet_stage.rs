//! The covered members of a local packet, one group at a time (R4,
//! 2026-10-28): every member is staged (its source recorded as it was
//! before the copy, as the single file's observer records it), one intent
//! record names each member's source, stage and destination, and only then
//! are they published, one after another, at their requested names. A crash
//! after the record leaves only stages the resumed job knows and removes; a
//! member that is not published (stop, failure) has its stage removed before
//! its receipt is given up.
use super::super::super::file_evidence::HostEvidence;
use super::super::super::file_recovery::{self, PacketIntent};
use super::super::super::job_store::snapshot;
use super::super::super::ops::{Meter, OpError};
use super::super::super::view::Side;
use super::super::super::Engine;
use super::{alone, settle, Member, PacketObserver, Tally};
use super::{Attempt, Failures};
use crate::copy::{LocalFailure, LocalOutcome, LocalRequest, PacketStage, PacketStaging};
use crate::transfer::walk_listers::native;
use crate::types::CopyMode;

/// Members staged before their intent at most: each keeps its stage's
/// handle open until it is published, so one worker holds at most this many
/// more open files (16 workers: 256).
pub(super) const STAGE_GROUP: usize = 16;

/// How a group ended.
pub(super) enum Ended {
    /// Every member of the group was decided; the packet goes on.
    Next,
    /// The job stops: the packet's remaining members are untried.
    Stopped,
    /// The intent could not be written: the remaining members go alone.
    Alone,
    /// A simulated crash (tests): nothing more happens.
    #[cfg_attr(not(test), allow(dead_code))]
    Crash,
}

/// A staged member waiting for its intent.
struct Staged {
    member: Member,
    source: HostEvidence,
    stage: PacketStage,
    /// Bytes the stage streamed (booked back when the member fails).
    moved: u64,
}

impl Staged {
    fn parts(self) -> (Member, PacketStage, u64) {
        (self.member, self.stage, self.moved)
    }
}

/// Stages, records and publishes one group of covered members; `index`
/// counts the packet's publications.
pub(super) fn run_group(
    engine: &Engine<'_>,
    group: Vec<Member>,
    meter: &Meter<'_>,
    failures: &mut Failures<'_, '_>,
    tally: &mut Tally,
    index: &mut usize,
) -> Ended {
    let mut staged: Vec<Staged> = Vec::with_capacity(group.len());
    let mut group = group.into_iter();
    while let Some(member) = group.next() {
        if engine.stopped() {
            give_up(engine, failures, staged.into_iter().map(Staged::parts));
            untried(failures, Some(member).into_iter().chain(group));
            return Ended::Stopped;
        }
        let guard = snapshot(Side::Local, &member.file.source);
        if !guard.verified || guard.directory || guard.link {
            alone(engine, member.file);
            continue;
        }
        let before = meter.moved();
        let result = stage(engine, &member, meter);
        let moved = meter.moved().saturating_sub(before);
        match result {
            Ok(PacketStaging::Staged(stage)) => staged.push(Staged {
                member,
                source: HostEvidence {
                    path: guard.path,
                    object: guard.object,
                    id: guard.id,
                    size: guard.size,
                    mtime_ms: guard.mtime_ms,
                },
                stage,
                moved,
            }),
            Ok(PacketStaging::Canceled) => {
                let canceled = Ok(LocalOutcome::Canceled);
                settle(engine, member.file, canceled, moved, meter, failures, tally);
                give_up(engine, failures, staged.into_iter().map(Staged::parts));
                untried(failures, group);
                return Ended::Stopped;
            }
            Err(failure) => {
                let failed = Err(failure);
                settle(engine, member.file, failed, moved, meter, failures, tally);
            }
        }
    }
    let Some(staged) = intend(engine, staged) else {
        return if engine.stopped() {
            Ended::Stopped
        } else {
            Ended::Alone
        };
    };
    let mut staged = staged.into_iter();
    while let Some(Staged {
        member,
        stage,
        moved,
        ..
    }) = staged.next()
    {
        let _current = *index;
        *index += 1;
        #[cfg(test)]
        if super::tests::crash_before(_current) {
            return Ended::Crash;
        }
        if engine.stopped() {
            let rest = staged.map(Staged::parts);
            give_up(
                engine,
                failures,
                std::iter::once((member, stage, moved)).chain(rest),
            );
            return Ended::Stopped;
        }
        let result = {
            let covered = Some(member.requested.as_path());
            let observer = PacketObserver::new(engine, &member.file, covered);
            meter.timed_target(|| stage.publish(engine.stop_flag(), &observer))
        };
        let canceled = matches!(result, Ok(LocalOutcome::Canceled));
        settle(engine, member.file, result, moved, meter, failures, tally);
        #[cfg(test)]
        super::tests::after(engine, _current);
        if canceled {
            give_up(engine, failures, staged.map(Staged::parts));
            return Ended::Stopped;
        }
    }
    Ended::Next
}

/// Copies `member` into its private stage next to its requested name.
fn stage(
    engine: &Engine<'_>,
    member: &Member,
    meter: &Meter<'_>,
) -> Result<PacketStaging, LocalFailure> {
    let source = native(&member.file.source);
    let request = LocalRequest {
        source: &source,
        target: &member.requested,
        conflict: engine.view.conflict,
        mode: CopyMode::Copy,
        cancel: engine.stop_flag(),
        parallel: 1,
        probe: None,
    };
    let mut progress = |bytes: u64| meter.add(bytes);
    meter.timed_target(|| crate::copy::stage_local_from(&request, &member.start, &mut progress))
}

/// The one durable intent of the staged members, each with its source, its
/// stage and its destination. A member whose stage or destination cannot be
/// described goes alone (its stage removed). `None` when the intent failed:
/// every stage is removed and, unless the job stops, every member goes alone
/// and meets the same problem on its own path, reported per file.
fn intend(engine: &Engine<'_>, staged: Vec<Staged>) -> Option<Vec<Staged>> {
    let mut kept = Vec::with_capacity(staged.len());
    let mut described = Vec::with_capacity(staged.len());
    for entry in staged {
        let proof = file_recovery::stage_proof(entry.stage.path(), entry.stage.handle());
        let destination = crate::copy::native_path_text(&entry.member.requested);
        match (proof, destination) {
            (Ok(proof), Ok(destination)) => {
                described.push((proof, destination));
                kept.push(entry);
            }
            _ => {
                remove_stage(engine, &entry.member, entry.stage);
                alone(engine, entry.member.file);
            }
        }
    }
    if kept.is_empty() {
        return Some(kept);
    }
    let intents: Vec<PacketIntent<'_>> = kept
        .iter()
        .zip(&described)
        .map(|(entry, (proof, destination))| PacketIntent {
            file: &entry.member.file,
            source: Some(&entry.source),
            existed_before: None,
            stage: Some(proof.clone()),
            destination: Some(destination.clone()),
        })
        .collect();
    let prepared = file_recovery::prepare_batch_known(engine, &intents);
    drop(intents);
    if prepared.is_ok() {
        return Some(kept);
    }
    let stopped = engine.stopped();
    for entry in kept {
        // Revoked first: a crash in between leaves a receipt without its
        // stage, never a stage without a receipt.
        let _ = file_recovery::unpublished(engine, &entry.member.file);
        remove_stage(engine, &entry.member, entry.stage);
        if !stopped {
            alone(engine, entry.member.file);
        }
    }
    None
}

/// Members that will not be published here: each stage goes first, then
/// the receipt is given up (the member is untried: a stopped job ends there,
/// otherwise it goes alone).
fn give_up(
    engine: &Engine<'_>,
    failures: &mut Failures<'_, '_>,
    staged: impl Iterator<Item = (Member, PacketStage, u64)>,
) {
    let mut members = Vec::new();
    for (member, stage, moved) in staged {
        remove_stage(engine, &member, stage);
        engine.stats.unmoved(moved);
        members.push(member);
    }
    untried(failures, members.into_iter());
}

fn untried(failures: &mut Failures<'_, '_>, members: impl Iterator<Item = Member>) {
    for member in members {
        failures.member(member.file, OpError::canceled(), Attempt::Untried);
    }
}

/// Removes a stage this packet made and will not publish; reported when it
/// cannot be removed (the source stays).
fn remove_stage(engine: &Engine<'_>, member: &Member, stage: PacketStage) {
    if let Err(failure) = stage.discard() {
        let error = match failure {
            LocalFailure::Source(error)
            | LocalFailure::Target(error)
            | LocalFailure::Publish(error) => error,
        };
        engine.issue(
            &member.file.source,
            &format!("Private Kopierstufe wurde nicht automatisch gelöscht: {error}"),
        );
    }
}
