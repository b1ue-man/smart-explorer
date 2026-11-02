//! Move command admission; source removal follows the confirmed receipt.
use super::super::link_paths::{link_move_refused, resolve_destination};
use super::super::target::Target;
use super::super::transfer::{transfer_plan, try_rename_fast};
use super::super::tree_observer::{TreeObserver, TreePhase};
use super::super::tree_ops::{
    copy_entry_with, remove_copied_source_verified, TreeOutcome, TreeRequest,
};
use super::super::{folder_conflict, transfer, tree_engine};
use super::print_notices;
use crate::types::CopyMode;

/// `se mv`: prints skipped entries and the source directories kept for them.
pub(crate) fn move_path(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
) -> Result<(), String> {
    move_with_folders(src, dst, recursive, force, None, None)
}

pub(crate) fn move_with_folders(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    folders: Option<crate::transfer::FolderConflict>,
    resume: Option<&str>,
) -> Result<(), String> {
    let outcome = move_observed_in_job(
        src,
        dst,
        recursive,
        force,
        &TreeObserver::default(),
        folders,
        resume,
    )?;
    print_notices(&outcome);
    if folders.is_some() || resume.is_some() {
        println!("{}", outcome.destination);
    }
    Ok(())
}

/// `se mv` for background callers. A cancellation that arrives after the
/// copy completed keeps the source: the removal phase is never interrupted
/// halfway. A source link is renamed, never followed.
pub(in crate::cli) fn move_observed(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
) -> Result<TreeOutcome, String> {
    move_observed_with_folders(
        src,
        dst,
        recursive,
        force,
        observer,
        crate::transfer::FolderConflict::Merge,
    )
}

pub(in crate::cli) fn move_observed_with_folders(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
    folders: crate::transfer::FolderConflict,
) -> Result<TreeOutcome, String> {
    move_observed_in_job(src, dst, recursive, force, observer, Some(folders), None)
}

pub(in crate::cli) fn move_observed_in_job(
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
    if let Some(id) = resume {
        if let Some(destination) = folder_conflict::CommandContext::finish_confirmed_move(
            src, dst, recursive, force, folders, id,
        )? {
            observer.set_phase(TreePhase::Done);
            return Ok(TreeOutcome::complete(destination));
        }
    }
    let (meta, dst_path) = transfer_plan(src, dst)?;
    if meta.is_dir && !recursive {
        return Err(format!("{} is a directory; pass --recursive", src.path));
    }
    let mut context = folder_conflict::CommandContext::begin(
        src, dst, &meta, dst_path, recursive, force, true, folders, resume,
    )?;
    let dst_path = context.record.destination.clone();
    let folders = context.record.folders;
    let recovery = if resume.is_some() {
        crate::transfer::RecoveryMode::Resume
    } else {
        crate::transfer::RecoveryMode::New
    };
    let alias_required = if meta.is_dir && folders != crate::transfer::FolderConflict::Merge {
        context.prepare_alias()?
    } else {
        false
    };
    transfer::validate_transfer_path(src, dst, &meta, &dst_path)?;
    let moved_root = if meta.is_dir
        && !meta.is_symlink
        && folders != crate::transfer::FolderConflict::Merge
        && src.backend.is_local()
        && dst.backend.is_local()
        && src.same_namespace(dst)
        && src.can_rename_with(dst)
    {
        Some(
            crate::transfer::engine::folder_reservation::reserve_named_move(
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
            .map_err(|error| {
                format!("cannot reserve/move destination directory {dst_path}: {error}")
            })?,
        )
    } else {
        None
    };
    if let Some((path, _, true)) = &moved_root {
        observer.set_phase(TreePhase::Done);
        context.finish()?;
        return Ok(TreeOutcome::complete(path.clone()));
    }
    let reserved = if moved_root.is_none()
        && meta.is_dir
        && !meta.is_symlink
        && folders != crate::transfer::FolderConflict::Merge
    {
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
    let dst_path = moved_root
        .as_ref()
        .map(|(path, _, _)| path.as_str())
        .or_else(|| reserved.as_ref().map(|(path, _)| path.as_str()))
        .unwrap_or(dst_path.as_str());
    if try_rename_fast(src, dst, &meta, &dst_path, force)? {
        observer.set_phase(TreePhase::Done);
        context.finish()?;
        return Ok(TreeOutcome::complete(dst_path.to_string()));
    }
    if meta.is_symlink {
        return Err(link_move_refused(&src.path));
    }
    let request = TreeRequest {
        recursive,
        force,
        resume: resume.is_some(),
        expected_root: Some(&meta),
        destination_check: None,
        observer,
    };
    // Across volumes (or into an existing folder) between local paths: the
    // planned tree as the engine's journaled move (M16).
    let engine = tree_engine::transfer_tree(src, dst, dst_path, &request, CopyMode::Move)?;
    if let Some(outcome) = engine {
        context.finish()?;
        return Ok(outcome);
    }
    let receipt = copy_entry_with(&*src.backend, &src.path, &*dst.backend, dst_path, &request)?;
    observer.check()?;
    observer.set_phase(TreePhase::RemovingSource);
    let kept = remove_copied_source_verified(&*src.backend, &*dst.backend, &receipt, observer)?;
    observer.set_phase(TreePhase::Done);
    context.finish()?;
    Ok(TreeOutcome {
        destination: receipt.destination().to_string(),
        omitted: receipt.omissions(),
        kept,
        update: None,
    })
}
