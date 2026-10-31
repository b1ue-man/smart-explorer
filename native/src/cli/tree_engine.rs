//! Planned local trees on the transfer engine (M16, 2026-10-28): `se cp`
//! into an existing folder or with `--force`, the MCP `copy` into a reserved
//! "keep both" folder, and `se mv`/MCP `move` that cannot be one rename, when
//! source and destination are local paths. They used to read every file into
//! an encrypted spool with a sync per file and then write it out again on one
//! thread (`tree_spool`, `tree_apply`); here the engine copies with its flows,
//! kernel copies and workers.
//!
//! What stays: the whole tree is planned and checked first
//! (`TransferPlan::build`: every destination conflict, name, link and special
//! file) and checked again before the first folder is created; nothing is
//! replaced without `--force`; a file that appears at the destination after
//! the check is left alone and ends the command as an error; links and
//! special files are omissions; folders that still hold omitted entries stay
//! at the source of a move. A move runs as the engine's journaled move:
//! every source is quarantined, copied durably or renamed, and removed only
//! after the receipt of its publication; an interrupted move is in the
//! recovery catalog instead of a spool.
//!
//! What differs (as on the plain engine path, `engine_copy`): sources are not
//! read in full before the first destination file is written, and a failing
//! file does not stop the others. A resumed command (`--resume`) stays on the
//! spool path, which compares existing destinations by content.
use super::engine_copy::{self, RunOptions};
use super::ops::parent_of;
use super::target::Target;
use super::tree_observer::TreePhase;
use super::tree_ops::{Omission, TreeOutcome, TreeRequest};
use super::tree_plan::TransferPlan;
use crate::transfer::engine_names::validate_pair_rels;
use crate::transfer::{JobItems, PairItem};
use crate::types::{Conflict, CopyMode};

/// Whether this path takes the command: local to local, a new run (no
/// `--resume`), engine path enabled, and a source whose read can never ask
/// for access (the terminal and the MCP service must not prompt).
fn applicable(src: &Target, dst: &Target, request: &TreeRequest<'_>) -> bool {
    if request.resume || !engine_copy::enabled_now() {
        return false;
    }
    if !src.backend.is_local() || !dst.backend.is_local() {
        return false;
    }
    match request.expected_root {
        Some(root) => !engine_copy::access_prompt_possible(src, root),
        None => false,
    }
}

/// Copies (`CopyMode::Copy`) or moves (`CopyMode::Move`) `src` to `dst_path`
/// on the engine. `Ok(None)`: not this path's case, nothing was touched.
pub(super) fn transfer_tree(
    src: &Target,
    dst: &Target,
    dst_path: &str,
    request: &TreeRequest<'_>,
    mode: CopyMode,
) -> Result<Option<TreeOutcome>, String> {
    if !applicable(src, dst, request) {
        return Ok(None);
    }
    let observer = request.observer;
    observer.set_phase(TreePhase::Planning);
    let mut plan = TransferPlan::build(
        &*src.backend,
        &src.path,
        &*dst.backend,
        dst_path,
        request.recursive,
        request.force,
        false,
        request.expected_root,
        observer,
    )?;
    if let (Some(check), Some(root)) = (request.destination_check, plan.entries.first()) {
        check(&root.destination)?;
    }
    let Some(job) = engine_job(&plan)? else {
        return Ok(None);
    };
    let (files, directories, bytes) = totals(&plan);
    observer.planned(files, directories, bytes);
    // The final read-only checks and every planned folder: the engine finds
    // its folders in place (merge) and publishes files only.
    super::tree_apply::prepare_directories(&*src.backend, &*dst.backend, &mut plan)?;
    let destination = job.root.clone();
    if !job.pairs.is_empty() {
        observer.begin_engine();
        observer.set_phase(TreePhase::Writing);
        let options = RunOptions {
            conflict: if request.force {
                Conflict::Overwrite
            } else {
                Conflict::Skip
            },
            mode,
            job_id: (mode == CopyMode::Move).then(crate::transfer::new_job_id),
        };
        let items = JobItems::Pairs(job.pairs);
        let finished = engine_copy::run_job(src, dst, &job.target_dir, &items, observer, &options)?;
        engine_copy::settle(&finished, &destination)?;
    }
    let kept = if mode == CopyMode::Move {
        observer.check()?;
        observer.set_phase(TreePhase::RemovingSource);
        remove_moved_folders(&*src.backend, &plan)?
    } else {
        Vec::new()
    };
    observer.set_phase(TreePhase::Done);
    Ok(Some(TreeOutcome {
        destination,
        omitted: plan
            .omitted
            .iter()
            .map(|omitted| Omission {
                path: omitted.source_path.clone(),
                kind: omitted.kind,
            })
            .collect(),
        kept,
        update: None,
    }))
}

/// The files of a plan as one engine job.
struct EngineJob {
    /// Destination of the planned root.
    root: String,
    /// The folder the pairs' relative names start in.
    target_dir: String,
    pairs: Vec<PairItem>,
}

/// The plan's files as pairs below the root's folder (a folder root: the
/// root itself; a file root: its parent). `None` when a relative name is
/// one the engine refuses or leaves out (app trash folders): the spool path
/// decides with its own messages and copies those too.
fn engine_job(plan: &TransferPlan) -> Result<Option<EngineJob>, String> {
    let root = plan
        .entries
        .first()
        .ok_or_else(|| "transfer plan unexpectedly contains no root".to_string())?;
    let root_path = root.destination_path.clone();
    let (target_dir, prefix) = if root.source.is_dir {
        let prefix = if root_path.ends_with('/') {
            root_path.clone()
        } else {
            format!("{root_path}/")
        };
        (root_path.clone(), Some(prefix))
    } else {
        let Some(parent) = parent_of(&root_path) else {
            return Ok(None);
        };
        (parent, None)
    };
    let mut pairs = Vec::new();
    for entry in plan.entries.iter().filter(|entry| !entry.source.is_dir) {
        let rel = match &prefix {
            Some(prefix) => match entry.destination_path.strip_prefix(prefix.as_str()) {
                Some(rel) => rel.to_string(),
                None => return Ok(None),
            },
            None => last_name(&entry.destination_path).to_string(),
        };
        if rel.split('/').any(crate::apptrash::excluded_name) {
            return Ok(None);
        }
        pairs.push(PairItem {
            source: entry.source_path.clone(),
            rel,
            size: Some(entry.source.size),
            mtime_ms: entry.source.mtime_ms,
            id: entry.source.id.clone(),
        });
    }
    if validate_pair_rels(pairs.iter().map(|pair| pair.rel.as_str()), false).is_err() {
        return Ok(None);
    }
    Ok(Some(EngineJob {
        root: root_path,
        target_dir,
        pairs,
    }))
}

fn last_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// Files, folders and bytes of a plan (the observer's totals).
fn totals(plan: &TransferPlan) -> (u64, u64, u64) {
    plan.entries
        .iter()
        .fold((0u64, 0u64, 0u64), |(files, folders, bytes), entry| {
            if entry.source.is_dir {
                (files, folders + 1, bytes)
            } else {
                (files + 1, folders, bytes + entry.source.size)
            }
        })
}

/// After a successful engine move every planned file left its source
/// folder; the folders go bottom-up. A folder that still holds an omitted
/// link or special file (or such a folder below it) stays and is returned,
/// root first. A folder that is no plain folder any more, or that gained an
/// entry (its removal fails: it is not empty), ends the command as an error
/// and keeps everything that is left.
fn remove_moved_folders(
    backend: &dyn crate::vfs::Backend,
    plan: &TransferPlan,
) -> Result<Vec<String>, String> {
    let keep = super::tree_remove::directories_holding_omissions(plan);
    let mut kept = Vec::new();
    for index in (0..plan.entries.len()).rev() {
        let entry = &plan.entries[index];
        if !entry.source.is_dir {
            continue;
        }
        if keep[index] {
            kept.push(entry.source_path.clone());
            continue;
        }
        let current = backend.stat(&entry.source_path).map_err(|error| {
            format!(
                "source changed before move cleanup: {}: {error}",
                entry.source_path
            )
        })?;
        if current.is_symlink || !current.is_dir {
            return Err(format!(
                "moved source directory changed before cleanup: {}",
                entry.source_path
            ));
        }
        backend.remove_dir(&entry.source_path).map_err(|error| {
            format!(
                "cannot remove moved source directory {}: {error}",
                entry.source_path
            )
        })?;
    }
    kept.reverse();
    Ok(kept)
}

#[cfg(test)]
#[path = "tree_engine_tests.rs"]
mod tests;
