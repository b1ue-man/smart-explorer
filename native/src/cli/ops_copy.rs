//! Copy command admission and P14 folder/recovery decisions.
use super::super::link_paths::{copy_remote_link, resolve_destination, source_root, SourceRoot};
use super::super::target::Target;
use super::super::transfer::{transfer_plan, transfer_plan_as};
use super::super::tree_observer::{TreeObserver, TreePhase};
use super::super::tree_ops::{copy_entry_with, TreeOutcome, TreeRequest};
use super::super::{engine_copy, folder_conflict, transfer, tree_engine};
use super::print_notices;
use crate::types::CopyMode;
use std::time::Instant;

/// `se cp` without `--stats` (the existing tests call it this way).
#[cfg(test)]
pub(crate) fn copy(src: &Target, dst: &Target, recursive: bool, force: bool) -> Result<(), String> {
    copy_with_stats(src, dst, recursive, force, false)
}

/// `se cp`, `se get`, `se put`: prints one stderr line per skipped link or
/// special file, and with `stats` one line with the rate and the limiting
/// resource after a successful copy.
pub(crate) fn copy_with_stats(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    stats: bool,
) -> Result<(), String> {
    copy_with_folder_stats(src, dst, recursive, force, stats, None, None)
}

pub(crate) fn copy_with_folder_stats(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    stats: bool,
    folders: Option<crate::transfer::FolderConflict>,
    resume: Option<&str>,
) -> Result<(), String> {
    let observer = TreeObserver::default();
    let started = Instant::now();
    let outcome = copy_observed_in_job(src, dst, recursive, force, &observer, folders, resume)?;
    print_notices(&outcome);
    if folders.is_some() || resume.is_some() {
        println!("{}", outcome.destination);
    }
    if stats {
        let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        engine_copy::print_stats(&observer, wall_ms);
    }
    Ok(())
}

/// `se cp` for background callers: the same checks and pipeline with
/// cancellation and progress.
pub(in crate::cli) fn copy_observed(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
) -> Result<TreeOutcome, String> {
    copy_observed_with_folders(
        src,
        dst,
        recursive,
        force,
        observer,
        crate::transfer::FolderConflict::Merge,
    )
}

pub(in crate::cli) fn copy_observed_with_folders(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
    folders: crate::transfer::FolderConflict,
) -> Result<TreeOutcome, String> {
    copy_observed_in_job(src, dst, recursive, force, observer, Some(folders), None)
}

pub(in crate::cli) fn copy_observed_in_job(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
    folders: Option<crate::transfer::FolderConflict>,
    resume: Option<&str>,
) -> Result<TreeOutcome, String> {
    let resolved_dst = resolve_destination(dst)?;
    let dst = resolved_dst.as_ref().unwrap_or(dst);
    let meta = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    let (resolved_src, name) = match source_root(src, &meta)? {
        SourceRoot::Plain => (None, None),
        SourceRoot::Resolved { target, name } => (Some(target), Some(name)),
        SourceRoot::RemoteLink => {
            let (_, dst_path) = transfer_plan(src, dst)?;
            return copy_remote_link(src, dst, &dst_path, force, observer);
        }
    };
    let plain_source = resolved_src.is_none();
    let src = resolved_src.as_ref().unwrap_or(src);
    let (source_snapshot, dst_path) = transfer_plan_as(src, dst, name.as_deref())?;
    if source_snapshot.is_dir && !recursive {
        return Err(format!("{} is a directory; pass --recursive", src.path));
    }
    let mut context = folder_conflict::CommandContext::begin(
        src,
        dst,
        &source_snapshot,
        dst_path,
        recursive,
        force,
        false,
        folders,
        resume,
    )?;
    let dst_path = context.record.destination.clone();
    let folders = context.record.folders;
    let recovery = if resume.is_some() {
        crate::transfer::RecoveryMode::Resume
    } else {
        crate::transfer::RecoveryMode::New
    };
    let alias_required =
        if source_snapshot.is_dir && folders != crate::transfer::FolderConflict::Merge {
            context.prepare_alias()?
        } else {
            false
        };
    transfer::validate_transfer_path(src, dst, &source_snapshot, &dst_path)?;
    let reserved = if source_snapshot.is_dir && folders != crate::transfer::FolderConflict::Merge {
        Some(
            crate::transfer::engine::folder_reservation::reserve_named_folder(
                &*src.backend,
                &src.path,
                &*dst.backend,
                &dst_path,
                folders == crate::transfer::FolderConflict::KeepBoth,
                &context.record.id,
                recovery,
                alias_required,
                observer.cancel_flag(),
            )
            .map_err(|error| format!("cannot reserve destination directory {dst_path}: {error}"))?,
        )
    } else {
        None
    };
    let dst_path = reserved
        .as_ref()
        .map_or(dst_path.as_str(), |(path, _)| path.as_str());
    // A plain source copied to a new name runs on the transfer engine; a
    // resolved link, an existing destination or any doubt stays on this path.
    if plain_source {
        if let Some(outcome) = engine_copy::try_copy(
            src,
            &meta,
            &source_snapshot,
            dst,
            dst_path,
            recursive,
            observer,
        )? {
            context.finish()?;
            return Ok(outcome);
        }
    }
    let request = TreeRequest {
        recursive,
        force,
        resume: resume.is_some(),
        expected_root: Some(&source_snapshot),
        destination_check: None,
        observer,
    };
    // An existing destination folder, `--force` or a reserved "keep both"
    // folder between local paths: the planned tree on the engine (M16).
    if plain_source {
        let engine = tree_engine::transfer_tree(src, dst, dst_path, &request, CopyMode::Copy)?;
        if let Some(outcome) = engine {
            context.finish()?;
            return Ok(outcome);
        }
    }
    let receipt = copy_entry_with(&*src.backend, &src.path, &*dst.backend, dst_path, &request)?;
    observer.set_phase(TreePhase::Done);
    context.finish()?;
    Ok(TreeOutcome {
        destination: receipt.destination().to_string(),
        omitted: receipt.omissions(),
        kept: Vec::new(),
        update: None,
    })
}
