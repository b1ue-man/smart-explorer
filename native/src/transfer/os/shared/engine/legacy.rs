//! The older worker entry points (`upload_paths_progress` and friends) keep
//! their signatures and run as engine jobs.
use super::super::engine_names::parent_path;
use super::super::job::{JobItems, Layout};
use super::super::types::TransferMsg;
use super::{first_source, run_view, JobView, Side};
use crate::types::{Conflict, CopyMode, FilterDef};
use std::sync::atomic::AtomicBool;

/// The older worker entry points (borrowed backends, whole selected entries
/// or explicit pairs, "keep both" on name conflicts) as engine jobs.
pub(crate) fn run_legacy(
    source: Side<'_>,
    target: Side<'_>,
    target_dir: &str,
    items: JobItems,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let id = super::new_job_id();
    let folders = crate::transfer::FolderConflictPolicy::new(if target.is_local() { crate::transfer::FolderConflict::Merge } else { crate::transfer::FolderConflict::KeepBoth });
    run_with_recovery(source, target, target_dir, items, filter, &id, &folders, crate::transfer::RecoveryMode::New, tx, cancel);
}

/// Recovery callers pass the original identity and decisions. Never infer
/// a previous job from matching paths: that could consume a new user copy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_legacy_with_identity(
    source: Side<'_>, target: Side<'_>, target_dir: &str, items: JobItems,
    filter: Option<(FilterDef, String)>, id: &str,
    folders: &crate::transfer::FolderConflictPolicy,
    tx: &crossbeam_channel::Sender<TransferMsg>, cancel: &AtomicBool,
) {
    run_with_recovery(source, target, target_dir, items, filter, id, folders, crate::transfer::RecoveryMode::Resume, tx, cancel);
}

#[allow(clippy::too_many_arguments)]
fn run_with_recovery(
    source: Side<'_>, target: Side<'_>, target_dir: &str, items: JobItems,
    filter: Option<(FilterDef, String)>, id: &str,
    folders: &crate::transfer::FolderConflictPolicy,
    recovery: crate::transfer::RecoveryMode,
    tx: &crossbeam_channel::Sender<TransferMsg>, cancel: &AtomicBool,
) {
    let source_label = match source {
        Side::Remote(backend) => backend.root_display(),
        Side::Local => parent_path(first_source(&items)),
    };
    let view = JobView {
        recoverable: true,
        recovery,
        id: Some(id),
        folders,
        source,
        target,
        target_dir,
        items: &items,
        layout: Layout::Tree,
        filter: filter.as_ref(),
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        source_label: &source_label,
        target_label: target_dir,
        resume: None,
    };
    run_view(
        view,
        &|message| {
            let _ = tx.send(message);
        },
        cancel,
    );
}
