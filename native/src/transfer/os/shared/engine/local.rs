//! Local → local: each file through the copy module's safe single-file
//! transfer (kernel copy into a private stage, no-replace publication under
//! the conflict policy, quarantined moves). A move of whole unfiltered
//! folders on one volume is first tried as one no-replace rename; the rest
//! moves file by file and empty source folders are removed afterwards.
use super::super::engine_names::{base_name, parent_rel};
use super::super::walk_listers::native;
use super::ops::{At, Meter, OpError, OpResult, Outcome};
use super::queue::FileWork;
use super::roots::RootPlan;
use super::Engine;
use crate::copy::{transfer_local, LocalFailure, LocalOutcome, LocalRequest};
use crate::types::Conflict;
use std::path::PathBuf;

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
    let request = LocalRequest {
        source: &source,
        target: &target,
        conflict,
        mode: engine.view.mode,
        cancel: engine.stop_flag(),
    };
    let mut progress = |bytes: u64| meter.add(bytes);
    match transfer_local(&request, &mut progress) {
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
        let plain_dir = crate::local_access::symlink_metadata(&source).is_ok_and(|metadata| {
            metadata.is_dir() && !crate::local_access::metadata_is_link_like(&source, &metadata)
        });
        if !plain_dir {
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

/// Removes the source folders a per-file move emptied.
pub(super) fn prune_moved_dirs(engine: &Engine<'_>, plan: &RootPlan) {
    if !engine.is_move() {
        return;
    }
    let dirs: Vec<PathBuf> = std::mem::take(&mut *super::lock(&engine.moved_dirs))
        .iter()
        .map(|dir| native(dir))
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
