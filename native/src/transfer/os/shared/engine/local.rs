//! Local → local: each file through the copy module's safe single-file
//! transfer (kernel copy into a private stage, no-replace publication under
//! the conflict policy, quarantined moves). A move of whole unfiltered
//! folders on one volume is first tried as one no-replace rename; the rest
//! moves file by file and empty source folders are removed afterwards.
use super::super::engine_names::{base_name, parent_rel};
use super::super::flow::{classify_error, Flow, FlowPermit};
use super::super::flow_control::OpOutcome;
use super::super::walk_listers::native;
use super::ops::{At, Meter, OpError, OpResult, Outcome};
use super::queue::FileWork;
use super::roots::RootPlan;
use super::Engine;
use crate::copy::{transfer_local, LocalFailure, LocalOutcome, LocalRequest};
use crate::transfer::segments::{MIN_SEGMENT, SPLIT_FROM};
use crate::types::Conflict;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn copy_file(engine: &Engine<'_>, file: &FileWork, meter: &Meter<'_>) -> OpResult {
    let source = native(&file.source);
    let target = native(&engine.folders.path_of(&file.rel));
    let conflict = if engine.resume {
        if let Some(outcome) = super::download::existing(engine, &target, file.size)? {
            return Ok(outcome);
        }
        // Created meanwhile: left alone, never replaced (K8).
        Conflict::Skip
    } else {
        engine.view.conflict
    };
    // Permits the flows have to spare turn into threads of this one copy
    // (large file, nothing else waiting, flash drive); never waited for.
    let extras = take_extras(engine, file.size, &target);
    let request = LocalRequest {
        source: &source,
        target: &target,
        conflict,
        mode: engine.view.mode,
        cancel: engine.stop_flag(),
        parallel: 1 + extras.count(),
    };
    let mut progress = |bytes: u64| meter.add(bytes);
    // One kernel call moves the bytes: source and target cannot be told
    // apart, so the whole call is booked to the target.
    let result = meter.timed_target(|| run_transfer(&request, &mut progress));
    extras.finish(&result);
    match result {
        Ok(LocalOutcome::Completed { bytes, target }) => {
            // A rename moves the bytes without streaming them.
            meter.credit(bytes.saturating_sub(meter.moved()));
            if parent_rel(&file.rel).is_none() {
                if let Some(name) = target.file_name() {
                    engine
                        .folders
                        .record_alias(&file.rel, &name.to_string_lossy());
                }
            }
            Ok(Outcome::Done)
        }
        Ok(LocalOutcome::Skipped) => Ok(Outcome::Skipped),
        Ok(LocalOutcome::Canceled) => Err(OpError::canceled()),
        Err(LocalFailure::Source(error)) => Err(OpError::source(error)),
        Err(LocalFailure::Target(error)) => Err(OpError::target(error)),
        Err(LocalFailure::Publish(error)) => Err(OpError::at(At::Publish, error)),
    }
}

fn run_transfer(
    request: &LocalRequest<'_>,
    progress: &mut dyn FnMut(u64),
) -> Result<LocalOutcome, LocalFailure> {
    #[cfg(test)]
    extras_tests::record(request);
    transfer_local(request, progress)
}

/// How many threads beyond the one that holds the operation's permit may
/// copy ranges of a file: none unless it is a copy (a move stays one
/// rename or one durable copy) of at least two shortest transfer parts,
/// nothing else waits in the queue (another file would use the permit
/// better) and the drive is known to be flash; at most one per part beyond
/// the first, because every thread needs a part of at least `MIN_SEGMENT`.
fn extras_wanted(size: u64, is_move: bool, queued: usize, flash: impl FnOnce() -> bool) -> usize {
    if is_move || queued > 0 || size < SPLIT_FROM || !flash() {
        return 0;
    }
    usize::try_from(size / MIN_SEGMENT - 1).unwrap_or(usize::MAX)
}

/// Whether the drive under `target` is flash. Unknown is not flash: a
/// rotating drive loses to the head jumps of side-by-side ranges (windows
/// builds copy through `CopyFile2`, one call, so never).
fn drive_is_flash(target: &Path) -> bool {
    #[cfg(test)]
    if let Some(forced) = extras_tests::forced_rotation(target) {
        return forced == Some(false);
    }
    #[cfg(not(windows))]
    {
        crate::transfer::platform::volume_is_rotational(target) == Some(false)
    }
    #[cfg(windows)]
    {
        let _ = target;
        false
    }
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

fn take_extras(engine: &Engine<'_>, size: u64, target: &Path) -> Extras {
    let wanted = extras_wanted(size, engine.is_move(), engine.queue.view().queued, || {
        drive_is_flash(target)
    });
    take_permits(&engine.op_flows.0, engine.op_flows.1.as_ref(), wanted)
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
    plan.walk_roots.retain(|root| {
        if engine.stopped() || crate::apptrash::excluded_name(base_name(&root.path)) {
            return true;
        }
        let source = native(&root.path);
        if !plain_dir(&source) {
            return true;
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
    let dirs: Vec<PathBuf> = std::mem::take(&mut *super::lock(&engine.moved_dirs))
        .into_iter()
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
