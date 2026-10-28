//! Local packets (M3, 2026-10-28): small new local copies of a job with a
//! crash log share one durable intent record and one permit. A worker takes
//! up to `MAX_FILES` small files, decides each first name (`packet_start`),
//! writes one intent for every member whose requested name is free
//! (`file_recovery::prepare_batch_known`: one record, one group commit),
//! then copies and publishes the members one after another through the
//! single file's stage and publication (`copy::transfer_local_from`).
//!
//! Order. The intent comes before the first stage: the packet record holds
//! no stage evidence (`PacketIntent` has none), so writing it after staging
//! would only keep every member's stage handle open and leave up to a whole
//! packet of private stages behind a crash instead of the one in progress.
//! It is still durable before any member is published, which is the
//! invariant; a member that must be numbered ("keep both", requested name
//! taken) or whose requested name is taken by a race publishes behind an
//! intent of its own with its stage (`LocalObserver`).
//!
//! Parallelism (flow model, `flow.rs`/`flow_control.rs`). The flow's limit
//! is the number of operations that run at once; every running operation is
//! one worker copying one file, packet or not. A packet therefore holds one
//! permit: more permits would idle while its members run one after another
//! on one thread. It takes at most its fair share of the queue, `queued /
//! limit` files, so after it the queue still holds as much work per permit
//! for every other worker; a short queue (the start of discovery, the tail
//! of a job) runs single files. The controller measures goodput as bytes
//! plus a fixed weight per finished operation, so a packet credits that
//! weight once per member it finished: the controller sees the same work
//! whether files come alone or packed.
use super::super::file_evidence::{with_check_meter, HostEvidence};
use super::super::file_recovery::{self, PacketIntent};
use super::super::job_store::snapshot;
use super::super::ops::{self, nanos, At, Meter, OpError, Outcome};
use super::super::queue::{FileWork, Work};
use super::super::view::Side;
use super::super::worker::{finished, run_file};
use super::super::Engine;
use super::{Attempt, Failures};
use crate::copy::{LocalFailure, LocalOutcome, LocalRequest};
use crate::transfer::engine_names::parent_rel;
use crate::transfer::flow::classify_error;
use crate::transfer::flow_control::{OpOutcome, OP_WEIGHT_BYTES};
use crate::transfer::memory::reserve_memory;
use crate::transfer::walk_listers::native;
use crate::types::{Conflict, CopyMode};
use std::io;
use std::path::PathBuf;
use std::time::Instant;

/// Files of one local packet at most: the analysis' 64 (G1, M3); one intent
/// record of 64 receipts stays far below a log record's bound.
pub(super) const MAX_FILES: usize = 64;
/// Largest file that travels in a local packet: the small-file limit of the
/// remote packets' floor (1 MiB, K-E); larger files keep their own permit and
/// may take range threads.
pub(super) const SMALL_FILE: u64 = 1024 * 1024;
/// Bytes of one local packet at most: 64 files of 1 MiB. The bytes go
/// through the kernel, nothing is buffered for them; the bound only keeps
/// one packet's time short.
pub(super) const MAX_BYTES: u64 = 64 * SMALL_FILE;

/// Whether this job's files may travel in local packets: local to local, a
/// crash log (without one there is no intent to share, and a packet would
/// only hold its permit longer), new copies under "skip" or "keep both"
/// (a move secures every source with an intent of its own, a replacement
/// records what it replaces), and not "transfer missing files" (which
/// checks every existing destination alone).
pub(super) fn applies(engine: &Engine<'_>) -> bool {
    engine.view.source.is_local()
        && engine.view.target.is_local()
        && engine.job_record.is_some()
        && engine.view.mode == CopyMode::Copy
        && engine.view.conflict != Conflict::Overwrite
        && !engine.resume
        && !disabled()
}

/// More small files for a local packet with `file`, or `None` for a single
/// copy (a large or retried file, a queue too short to share).
pub(super) fn members(engine: &Engine<'_>, file: &FileWork) -> Option<Vec<FileWork>> {
    if file.size > SMALL_FILE || file.retried || file.alone {
        return None;
    }
    let take = fair_share(engine)?;
    if !super::resume_allows(engine, file) || !super::packet_source(engine, file) {
        return None;
    }
    let candidates = engine
        .queue
        .take_small(take, MAX_BYTES.saturating_sub(file.size), SMALL_FILE);
    super::admit(engine, candidates)
}

/// Members to take beyond the first for the job's flows and queue.
fn fair_share(engine: &Engine<'_>) -> Option<usize> {
    let mut limit = engine.op_flows.0.limit();
    if let Some(other) = &engine.op_flows.1 {
        limit = limit.min(other.limit());
    }
    take_beyond_first(engine.queue.view().queued, limit)
}

/// The files per permit a queue of `queued` holds besides the file in hand
/// (`(queued + 1) / limit`), at most a packet, minus that file; `None` when
/// the share is not two files.
fn take_beyond_first(queued: usize, limit: usize) -> Option<usize> {
    let share = queued.saturating_add(1) / limit.max(1);
    let take = share.min(MAX_FILES).saturating_sub(1);
    (take > 0).then_some(take)
}

/// A member and the name it is published at first.
struct Member {
    file: FileWork,
    requested: PathBuf,
    start: PathBuf,
}

impl Member {
    /// Covered by the packet's intent: published first at its requested name.
    fn covered(&self) -> bool {
        self.start == self.requested
    }
}

/// Runs one local packet (the members' folders exist; `batch::run`). The
/// intent's group commit is waited for before the permit is taken: it is no
/// work of the flow's volume, and other workers' copies go on meanwhile.
pub(super) fn run(engine: &Engine<'_>, files: Vec<FileWork>, buffer: &mut [u8]) {
    let Some(members) = first_names(engine, files) else {
        return;
    };
    if members.len() < 2 {
        for member in members {
            run_file(engine, member.file, buffer);
        }
        return;
    }
    #[cfg(test)]
    tests::ran(engine.folders.root(), members.len());
    let Some(members) = intend(engine, members) else {
        return;
    };
    let mut failures = Failures::new(engine);
    let memory_asked = Instant::now();
    let Some(reservation) = reserve_memory(ops::reservation(engine), engine.stop_flag()) else {
        untried(&mut failures, None, members.into_iter());
        return;
    };
    let memory_wait = nanos(memory_asked.elapsed());
    engine.stats.book_memory_wait(memory_wait);
    let permit_asked = Instant::now();
    let Some(permits) = engine.acquire() else {
        untried(&mut failures, None, members.into_iter());
        return;
    };
    let waited = memory_wait.saturating_add(nanos(permit_asked.elapsed()));
    let active = engine
        .stats
        .begin(&format!("{} kleine Dateien (Paket)", members.len()));
    if engine.flows_have_spare() {
        engine.queue.notify();
    }
    let meter = Meter::new(&permits, &engine.stats);
    meter.add_wait(waited);
    let tally = with_check_meter(engine.stats.check_meter(), || {
        run_members(engine, members, &meter, &mut failures)
    });
    // Every finished member counts as one operation for the controller; the
    // permit's own finish credits the first.
    if tally.done > 1 {
        let others = (tally.done as u64 - 1).saturating_mul(OP_WEIGHT_BYTES);
        meter.add_hidden(others);
    }
    let outcome = match (&tally.error, tally.done) {
        (Some(error), 0) => classify_error(error),
        _ => OpOutcome::Done,
    };
    permits.finish_with(outcome, meter.accounts());
    drop(active);
    drop(reservation);
    failures.finish();
}

/// Each file's first name, decided before the intent. A file that is
/// skipped is finished here; one whose name or source cannot be looked at
/// goes alone (its own path decides and reports). `None` when the job
/// stopped.
fn first_names(engine: &Engine<'_>, files: Vec<FileWork>) -> Option<Vec<Member>> {
    let mut members = Vec::with_capacity(files.len());
    for file in files {
        if engine.stopped() {
            return None;
        }
        let requested = native(&engine.folders.path_of(&file.rel));
        match crate::copy::packet_start(&requested, engine.view.conflict) {
            // The single path reports a source that is no regular file
            // before it looks at the destination; so does the packet.
            Ok(None) if regular_source(&file) => finished(engine, Outcome::Skipped),
            Ok(Some(start)) => members.push(Member {
                file,
                requested,
                start,
            }),
            Ok(None) | Err(_) => alone(engine, file),
        }
    }
    Some(members)
}

fn regular_source(file: &FileWork) -> bool {
    let source = native(&file.source);
    crate::local_access::symlink_metadata(&source).is_ok_and(|metadata| {
        metadata.is_file() && !crate::local_access::metadata_is_link_like(&source, &metadata)
    })
}

/// A file that leaves the packet and goes alone, never in a packet again.
fn alone(engine: &Engine<'_>, mut file: FileWork) {
    file.alone = true;
    engine.queue.push_front(Work::File(file));
}

/// The one durable intent of the covered members, each with the source as
/// it is before its copy (as the single path records it). A covered member
/// whose source cannot be recorded goes alone. `None` when the packet ends
/// here: the job stopped, or the intent failed (every member then goes
/// alone and meets the same problem on its own path, reported per file).
fn intend(engine: &Engine<'_>, members: Vec<Member>) -> Option<Vec<Member>> {
    let mut kept = Vec::with_capacity(members.len());
    let mut evidence = Vec::with_capacity(members.len());
    for member in members {
        if !member.covered() {
            kept.push(member);
            continue;
        }
        let guard = snapshot(Side::Local, &member.file.source);
        if !guard.verified || guard.directory || guard.link {
            alone(engine, member.file);
            continue;
        }
        evidence.push(HostEvidence {
            path: guard.path,
            object: guard.object,
            id: guard.id,
            size: guard.size,
            mtime_ms: guard.mtime_ms,
        });
        kept.push(member);
    }
    let covered: Vec<&Member> = kept.iter().filter(|member| member.covered()).collect();
    let intents: Vec<PacketIntent<'_>> = covered
        .iter()
        .zip(&evidence)
        .map(|(member, source)| PacketIntent {
            file: &member.file,
            source: Some(source),
            existed_before: None,
        })
        .collect();
    let prepared = file_recovery::prepare_batch_known(engine, &intents);
    drop(intents);
    if prepared.is_ok() {
        return Some(kept);
    }
    for member in covered {
        let _ = file_recovery::unpublished(engine, &member.file);
    }
    if !engine.stopped() {
        for member in kept {
            alone(engine, member.file);
        }
    }
    None
}

#[derive(Default)]
struct Tally {
    done: usize,
    /// The first failure, for the controller when nothing finished.
    error: Option<io::Error>,
}

/// Copies and publishes the members one after another.
fn run_members(
    engine: &Engine<'_>,
    members: Vec<Member>,
    meter: &Meter<'_>,
    failures: &mut Failures<'_, '_>,
) -> Tally {
    let mut tally = Tally::default();
    let mut members = members.into_iter().enumerate();
    // `_index` is read by the test hooks only.
    while let Some((_index, member)) = members.next() {
        #[cfg(test)]
        if tests::crash_before(_index) {
            return tally;
        }
        if engine.stopped() {
            untried(failures, Some(member), members.map(|(_, member)| member));
            return tally;
        }
        let before = meter.moved();
        let result = transfer(engine, &member, meter);
        let moved = meter.moved().saturating_sub(before);
        let canceled = matches!(result, Ok(LocalOutcome::Canceled));
        settle(
            engine,
            member.file,
            result,
            moved,
            meter,
            failures,
            &mut tally,
        );
        #[cfg(test)]
        tests::after(engine, _index);
        if canceled {
            untried(failures, None, members.map(|(_, member)| member));
            return tally;
        }
    }
    tally
}

/// Members the packet ended before: their receipts are given up (a stopped
/// job ends there; otherwise each goes alone).
fn untried(
    failures: &mut Failures<'_, '_>,
    first: Option<Member>,
    rest: impl Iterator<Item = Member>,
) {
    for member in first.into_iter().chain(rest) {
        failures.member(member.file, OpError::canceled(), Attempt::Untried);
    }
}

fn transfer(
    engine: &Engine<'_>,
    member: &Member,
    meter: &Meter<'_>,
) -> Result<LocalOutcome, LocalFailure> {
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
    let covered = member.covered().then_some(member.requested.as_path());
    let observer = PacketObserver::new(engine, &member.file, covered);
    let mut progress = |bytes: u64| meter.add(bytes);
    meter.timed_target(|| {
        crate::copy::transfer_local_from(&request, &member.start, &mut progress, &observer)
    })
}

/// What one member came to, booked like the single path (`local::copy_file`
/// and `worker::run_file`).
fn settle(
    engine: &Engine<'_>,
    file: FileWork,
    result: Result<LocalOutcome, LocalFailure>,
    moved: u64,
    meter: &Meter<'_>,
    failures: &mut Failures<'_, '_>,
    tally: &mut Tally,
) {
    let failure = match result {
        Ok(LocalOutcome::Completed { bytes, target }) => {
            // A rename moves the bytes without streaming them.
            meter.credit(bytes.saturating_sub(moved));
            if parent_rel(&file.rel).is_none() {
                if let Some(name) = target.file_name() {
                    engine
                        .folders
                        .record_alias(&file.rel, &name.to_string_lossy());
                }
            }
            tally.done += 1;
            finished(engine, Outcome::Done);
            return;
        }
        Ok(LocalOutcome::Skipped) => match file_recovery::unpublished(engine, &file) {
            Ok(()) => {
                finished(engine, Outcome::Skipped);
                return;
            }
            Err(error) => (OpError::at(At::Unknown, error), Attempt::MaybePublished),
        },
        Ok(LocalOutcome::Canceled) => (OpError::canceled(), Attempt::NotPublished),
        Err(LocalFailure::Source(error)) => (OpError::source(error), Attempt::NotPublished),
        Err(LocalFailure::Target(error)) => (OpError::target(error), Attempt::NotPublished),
        Err(LocalFailure::Publish(error)) => {
            (OpError::at(At::Publish, error), Attempt::MaybePublished)
        }
    };
    let (failure, attempt) = failure;
    engine.stats.unmoved(moved);
    if tally.error.is_none() {
        let kind = failure.error.kind();
        tally.error = Some(io::Error::new(kind, failure.error.to_string()));
    }
    // A refused read asks the access gate on the file's own path, as alone.
    if failure.at == At::Source
        && failure.error.kind() == io::ErrorKind::PermissionDenied
        && engine.gate.is_some()
    {
        match file_recovery::unpublished(engine, &file) {
            Ok(()) => alone(engine, file),
            Err(error) => failures.member(
                file,
                OpError::at(At::Unknown, error),
                Attempt::MaybePublished,
            ),
        }
        return;
    }
    failures.member(file, failure, attempt);
}

/// Packets off: only the comparison benchmarks switch them off.
fn disabled() -> bool {
    #[cfg(test)]
    {
        tests::PACKETS_OFF.load(std::sync::atomic::Ordering::Acquire)
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[path = "local_packet_observer.rs"]
mod observer;
use observer::PacketObserver;

#[cfg(test)]
#[path = "speed_task_local_packet_tests.rs"]
mod benches;
#[cfg(test)]
#[path = "local_packet_tests.rs"]
mod tests;
