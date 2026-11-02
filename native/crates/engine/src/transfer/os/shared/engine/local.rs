//! Local → local: each file through the copy module's safe single-file
//! transfer (kernel copy into a private stage, no-replace publication under
//! the conflict policy, quarantined moves). A move of whole unfiltered
//! folders on one volume is first tried as one no-replace rename; the rest
//! moves file by file and empty source folders are removed afterwards. A
//! large copy, and a large move to another volume (M15), takes the permits
//! the flows have to spare as range threads. Small new copies of a job with
//! a crash log travel in packets (`batch::local_packet`, M3).
use super::super::engine_names::{base_name, parent_rel};
use super::super::flow::{classify_error, Flow, FlowPermit};
use super::super::flow_control::OpOutcome;
use super::super::walk_listers::native;
use super::ops::{At, Meter, OpError, OpResult, Outcome};
use super::queue::FileWork;
use super::roots::RootPlan;
use super::Engine;
use crate::copy::{
    transfer_local_fresh, transfer_local_observed, transfer_local_replacing, LocalFailure,
    LocalOutcome, LocalRequest,
};
use crate::transfer::segments::SPLIT_FROM;
use crate::transfer::update_rule::Replacing;
use crate::types::Conflict;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[path = "local_plan.rs"]
mod plan;
use plan::{DeviceKey, Drive, Memory, Plan, Ticket, Verdicts};

/// What this process measured about the device pairs it copied between with
/// direct requests (files that go around the page cache). Buffered copies
/// are not measured (`plan::plan`).
static VERDICTS: Verdicts = Verdicts::new();

/// `parent_created`: this job created the file's folder (nothing of anyone
/// else can be inside), so a new copy under "skip" does not look at its
/// destination first (A7); the no-replace publication still refuses a name
/// taken meanwhile.
pub(super) fn copy_file(
    engine: &Engine<'_>,
    file: &FileWork,
    parent_created: bool,
    meter: &Meter<'_>,
) -> OpResult {
    let source = native(&file.source);
    let target = native(&engine.folders.path_of(&file.rel));
    let conflict = if engine.resume {
        if let Some(outcome) =
            super::download::existing(engine, &target, file.size, Conflict::Skip)?
        {
            return Ok(outcome);
        }
        // Created meanwhile: left alone, never replaced (K8).
        Conflict::Skip
    } else {
        engine.conflict_of(file)
    };
    // Permits the flows have to spare turn into threads of this one copy
    // (large file, nothing else waiting, a drive that is flash or measured to
    // gain from it); never waited for.
    let (extras, ticket) = take_extras(engine, file.size, &source, &target);
    // The verdict of the measured start of this copy, kept for its devices.
    let record = |helps: bool| {
        if let Some(ticket) = &ticket {
            ticket.record(helps);
        }
    };
    let request = LocalRequest {
        source: &source,
        target: &target,
        conflict,
        mode: engine.view.mode,
        cancel: engine.stop_flag(),
        parallel: 1 + extras.count(),
        probe: ticket.as_ref().map(|_| &record as &dyn Fn(bool)),
    };
    let mut progress = |bytes: u64| meter.add(bytes);
    // One kernel call moves the bytes: source and target cannot be told
    // apart, so the whole call is booked to the target.
    let observer = super::file_recovery::LocalObserver::new(engine, file);
    let fresh = parent_created && !engine.resume;
    // A replacement the file rule decided is checked against the target it
    // saw right before it is published (`copy::transfer_local_replacing`).
    let decided = file
        .replacing
        .as_ref()
        .filter(|_| conflict == Conflict::Overwrite && !engine.resume);
    let result =
        meter.timed_target(|| run_transfer(&request, decided, fresh, &mut progress, &observer));
    extras.finish(&result);
    match result {
        Ok(LocalOutcome::Completed {
            bytes,
            target: published,
        }) => {
            // A rename moves the bytes without streaming them.
            meter.credit(bytes.saturating_sub(meter.moved()));
            if decided.is_some() && published != target {
                // The target was saved again after the decision: the copy
                // was published beside it, nothing was replaced.
                engine.stats.replace_refused();
                engine.issues.note(
                    &file.source,
                    "Ersetzen nicht möglich (Ziel hat sich seit der Entscheidung geändert) – beide behalten",
                );
            }
            if parent_rel(&file.rel).is_none() {
                if let Some(name) = published.file_name() {
                    engine
                        .folders
                        .record_alias(&file.rel, &name.to_string_lossy());
                }
            }
            Ok(Outcome::Done)
        }
        Ok(LocalOutcome::Skipped) => {
            super::file_recovery::unpublished(engine, file)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            Ok(Outcome::Skipped)
        }
        Ok(LocalOutcome::Canceled) => {
            super::file_recovery::unpublished(engine, file)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            Err(OpError::canceled())
        }
        Err(LocalFailure::Source(error)) => {
            super::file_recovery::unpublished(engine, file)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            Err(OpError::source(error))
        }
        Err(LocalFailure::Target(error)) => {
            super::file_recovery::unpublished(engine, file)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            Err(OpError::target(error))
        }
        Err(LocalFailure::Publish(error)) => Err(OpError::at(At::Publish, error)),
    }
}

fn run_transfer(
    request: &LocalRequest<'_>,
    decided: Option<&Replacing>,
    fresh: bool,
    progress: &mut dyn FnMut(u64),
    observer: &dyn crate::copy::PublicationObserver,
) -> Result<LocalOutcome, LocalFailure> {
    #[cfg(test)]
    extras_tests::record(request);
    if let Some(decided) = decided {
        return transfer_local_replacing(request, decided, progress, observer);
    }
    if fresh {
        return transfer_local_fresh(request, progress, observer);
    }
    transfer_local_observed(request, progress, observer)
}

/// Whether a file of `size` bytes is copied around the page cache: at least a
/// quarter of the available memory (`plan::unbuffered`). Only the Linux and
/// Android copy goes by it; the Windows copy has no ranges and decides by
/// itself in `CopyFile2`, so the answer changes nothing there.
fn unbuffered_of(size: u64, target: &Path) -> bool {
    #[cfg(test)]
    if let Some(forced) = extras_tests::forced_unbuffered(target) {
        return forced;
    }
    let _ = target;
    plan::unbuffered(size, crate::transfer::available_memory())
}

/// Whether the drive under `target` is flash, rotating or cannot be told
/// (windows builds copy through `CopyFile2`, one call, so never in ranges).
fn drive_of(target: &Path) -> Drive {
    #[cfg(test)]
    if let Some(forced) = extras_tests::forced_rotation(target) {
        return Drive::of(forced);
    }
    #[cfg(not(windows))]
    {
        Drive::of(crate::transfer::platform::volume_is_rotational(target))
    }
    #[cfg(windows)]
    {
        let _ = target;
        Drive::Unknown
    }
}

/// The device pair of a copy, when it can be measured: both sides on block
/// devices (never network, FUSE or other virtual file systems, whose writes at
/// scattered offsets keep the older one-thread copy) and a platform whose copy
/// takes ranges.
fn device_key(source: &Path, target: &Path) -> Option<DeviceKey> {
    #[cfg(test)]
    if let Some(forced) = extras_tests::forced_key(target) {
        return forced;
    }
    #[cfg(not(windows))]
    {
        crate::transfer::platform::copy_device_pair(source, target)
    }
    #[cfg(windows)]
    {
        let _ = (source, target);
        None
    }
}

/// Whether a move between the device pair `key` copies instead of renaming
/// (M15): only a pair of two known, different block devices counts. A side
/// without a block device (network, FUSE, virtual) or a platform without
/// ranges (`device_key` is `None`) keeps the move on one thread, as before;
/// a rename that still fails across a bind mount on one device copies on
/// one thread as well.
fn crosses_volumes(key: Option<DeviceKey>) -> bool {
    key.is_some_and(|(source, target)| source != target)
}

/// The extra permits of one copy, taken before it and given back after it.
pub(super) struct Extras {
    permits: Vec<(FlowPermit, Option<FlowPermit>)>,
}

impl Extras {
    pub(super) fn count(&self) -> usize {
        self.permits.len()
    }

    /// Ends every extra with what the copy came to: done work feeds the
    /// controllers, a skipped or canceled file does not.
    pub(super) fn finish(self, result: &Result<LocalOutcome, LocalFailure>) {
        for (first, second) in self.permits {
            let outcome = match result {
                Ok(LocalOutcome::Completed { .. }) => Some(OpOutcome::Done),
                Ok(LocalOutcome::Skipped | LocalOutcome::Canceled) => None,
                Err(
                    LocalFailure::Source(error)
                    | LocalFailure::Target(error)
                    | LocalFailure::Publish(error),
                ) => Some(classify_error(error)),
            };
            for permit in std::iter::once(first).chain(second) {
                match outcome {
                    Some(outcome) => permit.finish(outcome),
                    None => permit.abandon(),
                }
            }
        }
    }
}

/// The permits for the threads of one copy and, when the drive has to be
/// measured first, the right to measure it (given back when the copy ends
/// without a verdict).
fn take_extras(
    engine: &Engine<'_>,
    size: u64,
    source: &Path,
    target: &Path,
) -> (Extras, Option<Ticket<'static>>) {
    let none = || Extras {
        permits: Vec::new(),
    };
    // The common small file asks for nothing: no look into the queue (a lock
    // every worker shares) and no permit taking.
    if size < SPLIT_FROM
        || crate::source_media::probe_regular_file(source).is_ok_and(|source| source.is_some())
    {
        return (none(), None);
    }
    let key = device_key(source, target);
    let renames = engine.is_move() && !crosses_volumes(key);
    if renames {
        return (none(), None);
    }
    let memory = || key.map_or(Memory::Unavailable, |key| VERDICTS.memory(key));
    let planned = plan::plan(
        size,
        renames,
        engine.queue.view().queued,
        unbuffered_of(size, target),
        || drive_of(target),
        memory,
    );
    let (flow, other) = (&engine.op_flows.0, engine.op_flows.1.as_ref());
    match planned {
        Plan::Single => (none(), None),
        Plan::Ranges(wanted) => (take_permits(flow, other, wanted), None),
        Plan::Probe(wanted) => {
            let Some(ticket) = key.and_then(|key| VERDICTS.claim(key)) else {
                return (none(), None);
            };
            let extras = take_permits(flow, other, wanted);
            // A measurement needs a second thread; without one the claim goes
            // back to the next copy.
            if extras.count() == 0 {
                return (extras, None);
            }
            (extras, Some(ticket))
        }
    }
}

/// Up to `wanted` permits that are free right now on the first flow and, when
/// the copy spans two different flows, on the second as well; a second flow
/// without a free permit returns the first one and ends the taking. Never
/// waits.
pub(super) fn take_permits(one: &Arc<Flow>, other: Option<&Arc<Flow>>, wanted: usize) -> Extras {
    let other = other.filter(|other| other.key() != one.key());
    let mut permits = Vec::new();
    while permits.len() < wanted {
        let Some(first) = one.try_acquire() else {
            break;
        };
        let second = match other {
            Some(other) => match other.try_acquire() {
                Some(second) => Some(second),
                None => {
                    first.abandon();
                    break;
                }
            },
            None => None,
        };
        permits.push((first, second));
    }
    Extras { permits }
}

/// Moves selected folders as a whole when that is one rename (same volume,
/// destination name free); those leave the walk. Others move file by file.
pub(super) fn move_whole_roots(engine: &Engine<'_>, plan: &mut RootPlan) {
    let eligible = engine.is_move()
        && engine.keeps_folders()
        && !engine.resume
        && engine.view.source.is_local()
        && engine.view.target.is_local();
    if !eligible {
        return;
    }
    let reservations = plan._reservations.as_ref();
    let resolved_roots = &mut plan.roots;
    plan.walk_roots.retain_mut(|root| {
        if engine.stopped() || crate::apptrash::excluded_name(base_name(&root.path)) {
            return true;
        }
        let source = native(&root.path);
        // Folder conflict reservations never apply to a selected regular file.
        // Keep that file in the ordinary per-file move path.
        if !plain_dir(&source) {
            return true;
        }
        let policy = engine.view.folders.for_root(&root.path);
        if policy != crate::transfer::FolderConflict::Merge {
            if let Some(reservations) = reservations {
                match reservations.move_root(
                    engine.folders.root(),
                    &root.path,
                    &root.rel,
                    policy == crate::transfer::FolderConflict::KeepBoth,
                    engine.stop_flag(),
                ) {
                    Ok((relative, moved)) => {
                        root.rel = relative;
                        if let Some(resolved) = resolved_roots
                            .iter_mut()
                            .find(|entry| entry.source == root.path)
                        {
                            resolved.planned = root.rel.clone();
                        }
                        if moved {
                            engine.stats.entry_moved_whole();
                            return false;
                        }
                        return true;
                    }
                    Err(error) => {
                        engine.issue(&root.path, &format!("Ordner verschieben: {error}"));
                        return false;
                    }
                }
            }
        }
        if let Some(parent) = parent_rel(&root.rel) {
            if engine.folders.ensure(parent, engine.stop_flag()).is_err() {
                return true;
            }
        }
        let target = native(&engine.folders.path_of(&root.rel));
        match crate::copy::move_folder(&source, &target) {
            Ok(()) => {
                engine.stats.entry_moved_whole();
                false
            }
            Err(_) => true,
        }
    });
}

/// Removes the source folders a per-file move emptied, only where the
/// folder exists at the target (a folder the target refused, or one a stop
/// left out, stays at the source even when empty).
pub(super) fn prune_moved_dirs(engine: &Engine<'_>, plan: &RootPlan) {
    if !engine.is_move() {
        return;
    }
    // Retain selected roots of an incomplete journaled move. Their object
    // guards must remain available while unfinished siblings are resumed.
    let keep_roots = engine.job_record.is_some()
        && (engine.issues.total() != 0
            || engine
                .user_cancel
                .load(std::sync::atomic::Ordering::Acquire));
    let dirs: Vec<PathBuf> = std::mem::take(&mut *super::lock(&engine.moved_dirs))
        .into_iter()
        .filter(|(source, _)| {
            !keep_roots || !plan.walk_roots.iter().any(|root| root.path == *source)
        })
        .filter(|(_, rel)| plain_dir(&native(&engine.folders.path_of(rel))))
        .map(|(source, _)| native(&source))
        .collect();
    if dirs.is_empty() {
        return;
    }
    let roots: Vec<PathBuf> = plan
        .walk_roots
        .iter()
        .map(|root| native(&root.path))
        .collect();
    for (path, detail) in crate::copy::prune_empty_dirs(&roots, &dirs) {
        engine.issue(&path, &detail);
    }
}

/// A folder that is no link, junction or other reparse point.
fn plain_dir(path: &std::path::Path) -> bool {
    crate::local_access::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir() && !crate::local_access::metadata_is_link_like(path, &metadata)
    })
}

#[cfg(test)]
#[path = "local_extras_tests.rs"]
mod extras_tests;
#[cfg(test)]
#[path = "local_move_extras_tests.rs"]
mod move_extras_tests;
