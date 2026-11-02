//! Tree deletion for protocols whose primitives are type-strict: `remove_file`
//! never removes a folder and `remove_dir` removes only an empty folder and
//! never follows a link. Each folder is listed once and no entry is `stat`ed;
//! leaves go to the backend in portions it may pipeline, and every folder is
//! removed after its whole subtree. Only entries listed as plain folders are
//! entered, so a link is always removed as one entry.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::delete::{
    canceled_report, delete_failure, join_child, validate_child_name, DeleteBudget, DeleteTarget,
    RecursiveDeleteFailure, RecursiveDeletePhase, RecursiveDeleteProgress, RecursiveDeleteReport,
    RecursiveDeleteStatus,
};
use super::{Backend, VfsResult};

/// Leaves handed to the remover at once: enough to keep a pipelining
/// protocol busy between two cancel checks, bounded for progress updates.
pub const TREE_LEAF_PORTION: usize = 256;

struct PlannedDir {
    path: String,
    leaves: Vec<DeleteTarget>,
}

/// Delete the plain folder `root` and everything below it. `remove_leaves`
/// removes a portion of non-folder entries (files, links, special files) and
/// returns exactly one result per leaf in order; it may keep several requests
/// in flight, and a slow remover may answer `Interrupted` for leaves it did
/// not start once `cancel` is set (the run then ends `Canceled`). The whole tree is listed before the first removal, within the
/// budget of the generic walk, so an unreadable or oversized subtree removes
/// nothing. The first failure stops the run with the counters reached.
pub fn remove_tree_listed<B, F>(
    backend: &B,
    root: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(RecursiveDeleteProgress),
    remove_leaves: F,
) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure>
where
    B: Backend + ?Sized,
    F: Fn(&[DeleteTarget]) -> Vec<VfsResult<()>>,
{
    let mut budget = DeleteBudget::default();
    let plan = match plan_tree(backend, root, cancel, &mut budget, progress) {
        Ok(Some(plan)) => plan,
        Ok(None) => return Ok(canceled_report(&budget, 0)),
        Err(error) => return Err(delete_failure(error, &budget, 0)),
    };
    let mut removed = 0u64;
    // Pre-order reversed: every folder comes after all of its descendants.
    for dir in plan.into_iter().rev() {
        for portion in dir.leaves.chunks(TREE_LEAF_PORTION) {
            if cancel.load(Ordering::Acquire) {
                return Ok(canceled_report(&budget, removed));
            }
            report(progress, &budget, removed, &portion[0].path);
            let results = remove_leaves(portion);
            if results.len() != portion.len() {
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    "tree delete got a wrong number of leaf results",
                );
                return Err(delete_failure(error, &budget, removed));
            }
            let mut first_error = None;
            for result in results {
                match result {
                    Ok(()) => removed = removed.saturating_add(1),
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            if let Some(error) = first_error {
                if error.kind() == io::ErrorKind::Interrupted && cancel.load(Ordering::Acquire) {
                    return Ok(canceled_report(&budget, removed));
                }
                return Err(delete_failure(error, &budget, removed));
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(canceled_report(&budget, removed));
        }
        report(progress, &budget, removed, &dir.path);
        if let Err(error) = backend.remove_dir(&dir.path) {
            return Err(delete_failure(error, &budget, removed));
        }
        removed = removed.saturating_add(1);
        report(progress, &budget, removed, &dir.path);
    }
    Ok(RecursiveDeleteReport {
        status: RecursiveDeleteStatus::Complete,
        planned: budget.entries,
        removed,
    })
}

fn plan_tree<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
    cancel: &AtomicBool,
    budget: &mut DeleteBudget,
    progress: &mut dyn FnMut(RecursiveDeleteProgress),
) -> VfsResult<Option<Vec<PlannedDir>>> {
    budget.record(root, 0)?;
    planning(progress, budget, root);
    let meta = backend.stat(root)?;
    if !meta.is_dir || meta.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("tree delete target is not a plain folder: {root}"),
        ));
    }
    let mut plan = Vec::new();
    let mut pending = vec![(root.to_string(), 0usize)];
    while let Some((path, depth)) = pending.pop() {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let children = backend.list_dir(&path)?;
        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "delete depth overflow"))?;
        let mut names = std::collections::HashSet::new();
        let mut leaves = Vec::new();
        let mut folders = Vec::new();
        for child in children {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            validate_child_name(&child.name)?;
            if !names.insert(child.name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "backend returned duplicate child name in {path}: {:?}",
                        child.name
                    ),
                ));
            }
            let child_path = join_child(&path, &child.name);
            budget.record(&child_path, child_depth)?;
            planning(progress, budget, &child_path);
            if child.is_dir && !child.is_symlink {
                folders.push(child_path);
            } else {
                leaves.push(DeleteTarget {
                    path: child_path,
                    id: child.id,
                    is_dir: false,
                    is_symlink: child.is_symlink,
                });
            }
        }
        plan.push(PlannedDir { path, leaves });
        pending.extend(folders.into_iter().rev().map(|path| (path, child_depth)));
    }
    Ok(Some(plan))
}

fn planning(progress: &mut dyn FnMut(RecursiveDeleteProgress), budget: &DeleteBudget, path: &str) {
    progress(RecursiveDeleteProgress {
        phase: RecursiveDeletePhase::Planning,
        planned: budget.entries,
        removed: 0,
        current: path.to_string(),
    });
}

fn report(
    progress: &mut dyn FnMut(RecursiveDeleteProgress),
    budget: &DeleteBudget,
    removed: u64,
    path: &str,
) {
    progress(RecursiveDeleteProgress {
        phase: RecursiveDeletePhase::Applying,
        planned: budget.entries,
        removed,
        current: path.to_string(),
    });
}
