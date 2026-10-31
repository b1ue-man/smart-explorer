//! `se cp`, `se get`, `se put`, `se mv` and the MCP `copy` with `--conflict
//! update` ("neuere ersetzen ältere"): the whole copy runs on the transfer
//! engine, which decides the rule per file (`transfer/os/shared/engine/
//! update.rs`). A destination folder that exists is merged into, a missing
//! one is created; nothing is checked for an existing destination file
//! beforehand because that is what the rule is for.
use super::engine_copy::{self, RunOptions};
use super::entry_kind::EntryKind;
use super::file_rule::RuleCounts;
use super::link_paths::{resolve_destination, source_root, SourceRoot};
use super::ops::parent_of;
use super::target::Target;
use super::transfer::{transfer_plan_as, validate_transfer_path};
use super::tree_observer::{TreeObserver, TreePhase};
use super::tree_ops::TreeOutcome;
use crate::transfer::{JobItems, PairItem};
use crate::types::{Conflict, CopyMode};
use std::io::Write;
use std::time::Instant;

/// The terminal command: the outcome's notices (skipped links, kept source
/// folders, what the rule decided) go to stderr, `stats` prints the rate.
pub(super) fn run_cli(
    src: &Target,
    dst: &Target,
    recursive: bool,
    mode: CopyMode,
    folders: Option<crate::transfer::FolderConflict>,
    stats: bool,
) -> Result<(), String> {
    let observer = TreeObserver::default();
    let started = Instant::now();
    let outcome = run(src, dst, recursive, mode, &observer, folders)?;
    let mut err = std::io::stderr().lock();
    for line in outcome.notices() {
        let _ = writeln!(err, "se: {line}");
    }
    drop(err);
    println!("{}", outcome.destination);
    if stats {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        engine_copy::print_stats(&observer, wall_ms);
    }
    Ok(())
}

/// Copies or moves `src` to `dst` under the rule; background callers (the
/// MCP jobs) pass their observer.
pub(super) fn run(
    src: &Target,
    dst: &Target,
    recursive: bool,
    mode: CopyMode,
    observer: &TreeObserver,
    folders: Option<crate::transfer::FolderConflict>,
) -> Result<TreeOutcome, String> {
    match folders {
        None | Some(crate::transfer::FolderConflict::Merge) => {}
        Some(_) => {
            return Err(
                "--conflict update merges into an existing destination folder; it cannot be combined with another --folder-conflict"
                    .to_string(),
            )
        }
    }
    let resolved_dst = resolve_destination(dst)?;
    let dst = resolved_dst.as_ref().unwrap_or(dst);
    let meta = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    let (resolved_src, name) = match source_root(src, &meta)? {
        SourceRoot::Plain => (None, None),
        SourceRoot::Resolved { target, name } => (Some(target), Some(name)),
        SourceRoot::RemoteLink => {
            return Err(format!(
                "{} is a link; --conflict update compares regular files and folders",
                src.path
            ))
        }
    };
    let src = resolved_src.as_ref().unwrap_or(src);
    let (snapshot, dst_path) = transfer_plan_as(src, dst, name.as_deref())?;
    if matches!(
        EntryKind::of(&snapshot),
        EntryKind::Link | EntryKind::Special
    ) {
        return Err(format!(
            "{} is a link or special file; --conflict update compares regular files and folders",
            src.path
        ));
    }
    if snapshot.is_dir && !recursive {
        return Err(format!("{} is a directory; pass --recursive", src.path));
    }
    validate_transfer_path(src, dst, &snapshot, &dst_path)?;
    if mode == CopyMode::Move && !(src.backend.is_local() && dst.backend.is_local()) {
        return Err(
            "--conflict update moves only between local paths; copy instead, the source stays"
                .to_string(),
        );
    }
    observer.begin_engine();
    observer.set_phase(TreePhase::Planning);
    let (items, target_dir) = if snapshot.is_dir {
        let Some(children) = engine_copy::child_paths(src, &dst_path)? else {
            return Err(format!(
                "{}: a name in the folder cannot be copied safely",
                src.path
            ));
        };
        match dst.backend.stat(&dst_path) {
            Ok(existing) if existing.is_dir && !existing.is_symlink => {}
            Ok(_) => return Err(format!("{dst_path} exists and is not a directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => dst
                .backend
                .mkdir_all(&dst_path)
                .map_err(|error| format!("cannot create directory {dst_path}: {error}"))?,
            Err(error) => return Err(format!("cannot look at {dst_path}: {error}")),
        }
        let items = JobItems::Roots {
            paths: children,
            base: Some(src.path.clone()),
        };
        (items, dst_path.clone())
    } else {
        let Some(parent) = parent_of(&dst_path) else {
            return Err(format!("{dst_path}: no destination folder"));
        };
        let pair = PairItem {
            source: src.path.clone(),
            rel: engine_copy::last_name(&dst_path).to_string(),
            size: Some(snapshot.size),
            mtime_ms: snapshot.mtime_ms,
            id: snapshot.id.clone(),
        };
        (JobItems::Pairs(vec![pair]), parent)
    };
    let mut outcome = if items.is_empty() {
        TreeOutcome::complete(dst_path.clone())
    } else {
        observer.set_phase(TreePhase::Writing);
        let options = RunOptions {
            conflict: Conflict::Update,
            mode,
            job_id: (mode == CopyMode::Move).then(crate::transfer::new_job_id),
        };
        let finished = engine_copy::run_job(src, dst, &target_dir, &items, observer, &options)?;
        let mut outcome = engine_copy::settle_merged(&finished, &dst_path)?;
        outcome.update = Some(RuleCounts::of(&finished.progress));
        outcome
    };
    if mode == CopyMode::Move && snapshot.is_dir {
        observer.set_phase(TreePhase::RemovingSource);
        // The files moved one by one: the folder goes when it is empty now;
        // equal and newer-at-the-destination files keep it.
        if src.backend.remove_dir(&src.path).is_err() {
            outcome.kept.push(src.path.clone());
        }
    }
    observer.set_phase(TreePhase::Done);
    Ok(outcome)
}

#[cfg(test)]
#[path = "update_copy_tests.rs"]
mod tests;
