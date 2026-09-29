//! One local file copied or moved into a prepared folder: private stage,
//! kernel copy, no-replace publication under the conflict policy, and a move
//! that relocates its source to an unpredictable quarantine first. New copies
//! skip the per-file disk sync; moves and replacements keep it.
use crate::types::{Conflict, CopyMode};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::platform;
use super::durability::{finish_direct_move, finish_staged_commit};
#[cfg(test)]
use super::move_guard::restore_quarantine;
use super::move_guard::{
    moved_target_changed_error, quarantine_source, remove_quarantine, restore_after_error,
    restore_quarantine_if_any, source_changed_error, source_snapshot_path,
};
#[cfg(test)]
use super::path_guard::prepare_target_parent;
use super::staging::{stage_copy, LocalFailure};

/// Numbered names tried for one file when others keep taking them.
const MAX_RENAME_RACE_RETRIES: usize = 1000;

/// One local file for the engine; the destination folder exists and was
/// checked to be a plain folder.
pub(crate) struct LocalRequest<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    pub conflict: Conflict,
    pub mode: CopyMode,
    pub cancel: &'a AtomicBool,
}

#[derive(Debug)]
pub(crate) enum LocalOutcome {
    /// Published at `target` (a numbered name under "keep both").
    Completed {
        bytes: u64,
        target: PathBuf,
    },
    Skipped,
    Canceled,
}

/// Copies or moves one file; `progress` receives streamed bytes.
pub(crate) fn transfer_local(
    request: &LocalRequest<'_>,
    progress: &mut dyn FnMut(u64),
) -> Result<LocalOutcome, LocalFailure> {
    let src = request.source;
    let source_metadata =
        crate::local_access::symlink_metadata(src).map_err(LocalFailure::Source)?;
    if platform::metadata_is_link_like(&source_metadata) || !source_metadata.is_file() {
        return Err(LocalFailure::Source(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("source is not a regular, non-link file: {}", src.display()),
        )));
    }
    let requested = request.target.to_path_buf();
    let Some(mut target) =
        select_initial_target(src, &requested, request.conflict).map_err(LocalFailure::Target)?
    else {
        return Ok(LocalOutcome::Skipped);
    };
    if request.cancel.load(Ordering::Acquire) {
        return Ok(LocalOutcome::Canceled);
    }
    let mut races = 0usize;
    // A move first relocates the source to an unpredictable no-replace
    // sibling; no check-then-unlink ever targets the user-visible source.
    let mut quarantine = if request.mode == CopyMode::Move {
        Some(quarantine_source(src).map_err(LocalFailure::Source)?)
    } else {
        None
    };
    if request.cancel.load(Ordering::Acquire) {
        restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
        return Ok(LocalOutcome::Canceled);
    }
    let source_path = quarantine
        .as_ref()
        .map(|source| source.path.clone())
        .unwrap_or_else(|| src.to_path_buf());

    if request.mode == CopyMode::Move {
        loop {
            match platform::move_file(
                &source_path,
                &target,
                request.conflict == Conflict::Overwrite,
            ) {
                Ok(()) => {
                    let source = quarantine.take().ok_or_else(|| {
                        LocalFailure::Publish(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "move quarantine is missing",
                        ))
                    })?;
                    match source_snapshot_path(&target) {
                        Ok(snapshot) if snapshot == source.snapshot => {}
                        Ok(_) => {
                            return Err(LocalFailure::Publish(moved_target_changed_error(
                                &target, None,
                            )))
                        }
                        Err(error) => {
                            return Err(LocalFailure::Publish(moved_target_changed_error(
                                &target,
                                Some(error),
                            )))
                        }
                    }
                    // POSIX rename may be a no-op when the destination is a
                    // hard link to the same inode. Remove only our sibling.
                    if metadata_if_exists(&source.path)
                        .map_err(LocalFailure::Publish)?
                        .is_some()
                    {
                        remove_quarantine(&source).map_err(LocalFailure::Publish)?;
                    }
                    finish_direct_move(&target, &source, platform::sync_parent)
                        .map_err(LocalFailure::Publish)?;
                    return Ok(LocalOutcome::Completed {
                        bytes: source.snapshot.len,
                        target,
                    });
                }
                Err(error) if platform::is_cross_device(&error) => break,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    match request.conflict {
                        Conflict::Skip => {
                            restore_quarantine_if_any(quarantine.as_ref())
                                .map_err(LocalFailure::Source)?;
                            return Ok(LocalOutcome::Skipped);
                        }
                        Conflict::Rename => {
                            target = next_name(&requested, &mut races).map_err(|error| {
                                LocalFailure::Publish(restore_after_error(
                                    quarantine.as_ref(),
                                    error,
                                ))
                            })?;
                        }
                        Conflict::Overwrite => {
                            return Err(LocalFailure::Publish(restore_after_error(
                                quarantine.as_ref(),
                                error,
                            )));
                        }
                    }
                }
                Err(error) => {
                    return Err(LocalFailure::Publish(restore_after_error(
                        quarantine.as_ref(),
                        error,
                    )))
                }
            }
        }
    }

    let durable = request.mode == CopyMode::Move || request.conflict == Conflict::Overwrite;
    let staged = match stage_copy(&source_path, &target, request.cancel, durable, progress) {
        Ok(Some(staged)) => staged,
        Ok(None) => {
            restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
            return Ok(LocalOutcome::Canceled);
        }
        Err(failure) => return Err(restore_failure(quarantine.as_ref(), failure)),
    };
    if request.cancel.load(Ordering::Acquire) {
        staged.discard();
        restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
        return Ok(LocalOutcome::Canceled);
    }
    let identity = platform::file_identity(&staged.file).map_err(LocalFailure::Target);
    let same = identity.and_then(|identity| {
        platform::path_matches_identity(&staged.temp, identity).map_err(LocalFailure::Target)
    });
    if !matches!(same, Ok(true)) {
        staged.discard();
        let error = io::Error::new(
            io::ErrorKind::InvalidData,
            "staged copy path changed before commit",
        );
        return Err(LocalFailure::Target(restore_after_error(
            quarantine.as_ref(),
            error,
        )));
    }
    if let Some(source) = &quarantine {
        if staged.snapshot != source.snapshot {
            staged.discard();
            return Err(LocalFailure::Source(restore_after_error(
                quarantine.as_ref(),
                source_changed_error(&source.path),
            )));
        }
    }

    loop {
        match platform::commit_staged(
            &staged.temp,
            &target,
            request.conflict == Conflict::Overwrite,
        ) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match request.conflict {
                Conflict::Skip => {
                    staged.discard();
                    restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
                    return Ok(LocalOutcome::Skipped);
                }
                Conflict::Rename => match next_name(&requested, &mut races) {
                    Ok(next) => target = next,
                    Err(error) => {
                        staged.discard();
                        return Err(LocalFailure::Publish(restore_after_error(
                            quarantine.as_ref(),
                            error,
                        )));
                    }
                },
                Conflict::Overwrite => {
                    staged.discard();
                    return Err(LocalFailure::Publish(restore_after_error(
                        quarantine.as_ref(),
                        error,
                    )));
                }
            },
            Err(error) => {
                staged.discard();
                return Err(LocalFailure::Publish(restore_after_error(
                    quarantine.as_ref(),
                    error,
                )));
            }
        }
    }
    let bytes = staged.bytes;
    drop(staged.file);
    if durable {
        finish_staged_commit(&target, src, &mut quarantine, platform::sync_parent)
            .map_err(LocalFailure::Publish)?;
    }
    Ok(LocalOutcome::Completed { bytes, target })
}

/// The quarantined move source goes back where it was when the copy failed.
fn restore_failure(
    quarantine: Option<&super::move_guard::QuarantinedSource>,
    failure: LocalFailure,
) -> LocalFailure {
    match failure {
        LocalFailure::Source(error) => LocalFailure::Source(restore_after_error(quarantine, error)),
        LocalFailure::Target(error) => LocalFailure::Target(restore_after_error(quarantine, error)),
        LocalFailure::Publish(error) => {
            LocalFailure::Publish(restore_after_error(quarantine, error))
        }
    }
}

/// "Keep both" starts at the requested name and only probes on a conflict;
/// "skip" and "overwrite" look at the destination first.
fn select_initial_target(
    src: &Path,
    target: &Path,
    conflict: Conflict,
) -> io::Result<Option<PathBuf>> {
    if conflict == Conflict::Rename {
        return Ok(Some(target.to_path_buf()));
    }
    let Some(metadata) = metadata_if_exists(target)? else {
        return Ok(Some(target.to_path_buf()));
    };
    if conflict == Conflict::Skip {
        return Ok(None);
    }
    if metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination is a directory: {}", target.display()),
        ));
    }
    if platform::metadata_is_link_like(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination is a link or reparse point: {}",
                target.display()
            ),
        ));
    }
    if platform::same_file(src, target)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and destination are the same file",
        ));
    }
    Ok(Some(target.to_path_buf()))
}

fn next_name(requested: &Path, races: &mut usize) -> io::Result<PathBuf> {
    *races = races.saturating_add(1);
    if *races > MAX_RENAME_RACE_RETRIES {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination names kept changing during conflict-safe rename",
        ));
    }
    unique_path(requested)
}

fn unique_path(target: &Path) -> io::Result<PathBuf> {
    if metadata_if_exists(target)?.is_none() {
        return Ok(target.to_path_buf());
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = target
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for index in 2..=100_000u32 {
        let candidate = parent.join(format!("{stem} ({index}){ext}"));
        if metadata_if_exists(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique conflict name",
    ))
}

fn metadata_if_exists(path: &Path) -> io::Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The older single-file entry point with its own folder preparation, kept
/// for the regression tests of the safety rules above.
#[cfg(test)]
#[derive(Debug)]
pub(super) enum TransferResult {
    Completed(u64),
    Skipped,
    Canceled,
}

#[cfg(test)]
pub(super) fn transfer_file(
    src: &Path,
    target: &Path,
    destination_root: &Path,
    conflict: Conflict,
    mode: CopyMode,
    cancel: &AtomicBool,
) -> io::Result<TransferResult> {
    prepare_target_parent(destination_root, target)?;
    let request = LocalRequest {
        source: src,
        target,
        conflict,
        mode,
        cancel,
    };
    match transfer_local(&request, &mut |_| {}) {
        Ok(LocalOutcome::Completed { bytes, .. }) => Ok(TransferResult::Completed(bytes)),
        Ok(LocalOutcome::Skipped) => Ok(TransferResult::Skipped),
        Ok(LocalOutcome::Canceled) => Ok(TransferResult::Canceled),
        Err(failure) => Err(failure.into_error()),
    }
}

#[cfg(test)]
#[path = "safe_file_tests.rs"]
mod tests;
