//! One local file copied or moved into a prepared folder: private stage,
//! kernel copy, no-replace publication under the conflict policy, and a move
//! that relocates its source to an unpredictable quarantine first. New copies
//! skip the per-file disk sync; moves and replacements keep it.
use crate::types::{Conflict, CopyMode};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::platform;
use super::durability::finish_staged_commit;
#[cfg(test)]
use super::move_guard::{quarantine_source, restore_quarantine, source_snapshot_path};
use super::move_guard::{
    quarantine_source_observed, restore_after_error, restore_quarantine_if_any, source_changed_error,
};
#[cfg(test)]
use super::path_guard::prepare_target_parent;
use super::staging::{stage_copy, LocalFailure, Parallel};
use super::publication::{self, PublicationObserver};

#[path = "direct_move.rs"]
mod direct_move;

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
    /// Threads that may copy this one file's byte ranges at once (1 = one).
    /// Only a new copy uses more: moves and replacements stay at one.
    pub parallel: usize,
    /// Set (with `parallel` > 1) for a drive not known to be flash: the copy
    /// measures whether ranges help and reports the verdict here; the rest of
    /// the file is copied accordingly. Ignored by moves and replacements.
    pub probe: Option<&'a dyn Fn(bool)>,
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
    transfer_local_inner(request, progress, None)
}

/// The same local transfer with durable receipt hooks around publication.
pub(crate) fn transfer_local_observed(
    request: &LocalRequest<'_>,
    progress: &mut dyn FnMut(u64),
    observer: &dyn PublicationObserver,
) -> Result<LocalOutcome, LocalFailure> {
    transfer_local_inner(request, progress, Some(observer))
}

fn transfer_local_inner(
    request: &LocalRequest<'_>,
    progress: &mut dyn FnMut(u64),
    observer: Option<&dyn PublicationObserver>,
) -> Result<LocalOutcome, LocalFailure> {
    let src = request.source;
    let source_metadata =
        crate::local_access::symlink_metadata(src).map_err(LocalFailure::Source)?;
    if crate::local_access::metadata_is_link_like(src, &source_metadata)
        || !source_metadata.is_file()
    {
        return Err(LocalFailure::Source(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("source is not a regular, non-link file: {}", src.display()),
        )));
    }
    let requested = request.target.to_path_buf();
    let Some(mut target) = select_initial_target(src, &requested, request.conflict)? else {
        return Ok(LocalOutcome::Skipped);
    };
    if request.cancel.load(Ordering::Acquire) {
        return Ok(LocalOutcome::Canceled);
    }
    let mut races = 0usize;
    // A move first relocates the source to an unpredictable no-replace
    // sibling; no check-then-unlink ever targets the user-visible source.
    let mut quarantine = if request.mode == CopyMode::Move {
        Some(quarantine_source_observed(src, observer).map_err(LocalFailure::Source)?)
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
        if let Some(outcome) = direct_move::try_move(
            request, &source_path, &requested, &mut target, &mut races, &mut quarantine, observer,
        )? {
            return Ok(outcome);
        }
    }

    let durable = request.mode == CopyMode::Move || request.conflict == Conflict::Overwrite;
    let parallel = if durable {
        Parallel {
            threads: 1,
            probe: None,
        }
    } else {
        Parallel {
            threads: request.parallel.max(1),
            probe: request.probe,
        }
    };
    let staged = match stage_copy(
        &source_path,
        &target,
        request.cancel,
        durable,
        request.conflict == Conflict::Overwrite,
        &parallel,
        progress,
    ) {
        Ok(Some(staged)) => staged,
        Ok(None) => {
            restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
            return Ok(LocalOutcome::Canceled);
        }
        Err(failure) => return Err(restore_failure(quarantine.as_ref(), failure)),
    };
    if request.cancel.load(Ordering::Acquire) {
        staged.discard().map_err(LocalFailure::Publish)?;
        restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
        return Ok(LocalOutcome::Canceled);
    }
    let identity = platform::file_identity(&staged.file).map_err(LocalFailure::Target);
    let same = identity.and_then(|identity| {
        platform::path_matches_identity(&staged.temp, identity).map_err(LocalFailure::Target)
    });
    if !matches!(same, Ok(true)) {
        staged.discard().map_err(LocalFailure::Publish)?;
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
            staged.discard().map_err(LocalFailure::Publish)?;
            return Err(LocalFailure::Source(restore_after_error(
                quarantine.as_ref(),
                source_changed_error(&source.path),
            )));
        }
    }

    loop {
        if let Err(error) = publication::before(observer, &staged.temp, &target) {
            publication::preparation_failed(observer, &staged.temp, &target, &error, quarantine.as_ref())
                .map_err(LocalFailure::Publish)?;
            staged.discard().map_err(LocalFailure::Publish)?;
            return Err(LocalFailure::Publish(restore_after_error(quarantine.as_ref(), error)));
        }
        let committed = platform::commit_staged(
            &staged.temp,
            &target,
            request.conflict == Conflict::Overwrite,
        );
        if let Err(error) = &committed {
            publication::commit_failed(observer, &staged.temp, &target, &staged.file,
                error, quarantine.as_ref()).map_err(LocalFailure::Publish)?;
        }
        match committed {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match request.conflict {
                Conflict::Skip => {
                    staged.discard().map_err(LocalFailure::Publish)?;
                    restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
                    return Ok(LocalOutcome::Skipped);
                }
                Conflict::Rename => match next_name(&requested, &mut races) {
                    Ok(next) => target = next,
                    Err(error) => {
                        staged.discard().map_err(LocalFailure::Publish)?;
                        return Err(LocalFailure::Publish(restore_after_error(
                            quarantine.as_ref(),
                            error,
                        )));
                    }
                },
                Conflict::Overwrite => {
                    staged.discard().map_err(LocalFailure::Publish)?;
                    return Err(LocalFailure::Publish(restore_after_error(
                        quarantine.as_ref(),
                        error,
                    )));
                }
            },
            Err(error) => {
                staged.discard().map_err(LocalFailure::Publish)?;
                return Err(LocalFailure::Publish(restore_after_error(
                    quarantine.as_ref(),
                    error,
                )));
            }
        }
    }
    let bytes = staged.bytes;
    if durable {
        crate::copy::publication_proof::confirm_handle_publication(&target, &staged.file, request.cancel)
            .map_err(|error| {
                let retained = quarantine.as_ref().map_or(String::new(), |source|
                    format!("; secured move source retained at {}", source.path.display()));
                LocalFailure::Publish(io::Error::new(error.kind(), format!(
                    "destination committed at {}{retained}; destination durability confirmation failed: {error}",
                    target.display(),
                )))
            })?;
    }
    publication::after(observer, &target, &staged.file, quarantine.as_ref()).map_err(LocalFailure::Publish)?;
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
/// "skip" and "overwrite" look at the destination first. A problem with the
/// one existing destination file is this file's failure, never the whole
/// target's; a source that cannot be read is the source's (a protected one
/// asks for read access).
fn select_initial_target(
    src: &Path,
    target: &Path,
    conflict: Conflict,
) -> Result<Option<PathBuf>, LocalFailure> {
    if conflict == Conflict::Rename {
        return Ok(Some(target.to_path_buf()));
    }
    let Some(metadata) = metadata_if_exists(target).map_err(LocalFailure::Target)? else {
        return Ok(Some(target.to_path_buf()));
    };
    if conflict == Conflict::Skip {
        return Ok(None);
    }
    if metadata.is_dir() {
        return Err(LocalFailure::Target(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination is a directory: {}", target.display()),
        )));
    }
    if crate::local_access::metadata_is_link_like(target, &metadata) {
        return Err(LocalFailure::Target(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination is a link or reparse point: {}",
                target.display()
            ),
        )));
    }
    if crate::local_access::metadata_class(target, &metadata).special || !metadata.is_file() {
        return Err(LocalFailure::Target(
            crate::local_access::NotRegular::Special.error(),
        ));
    }
    let source = platform::path_identity(src).map_err(LocalFailure::Source)?;
    let existing = platform::path_identity(target).map_err(|error| {
        LocalFailure::Publish(io::Error::new(
            error.kind(),
            format!(
                "{}: vorhandene Zieldatei nicht prüfbar: {error}",
                target.display()
            ),
        ))
    })?;
    if source == existing {
        return Err(LocalFailure::Target(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and destination are the same file",
        )));
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
    Completed,
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
        parallel: 1,
        probe: None,
    };
    match transfer_local(&request, &mut |_| {}) {
        Ok(LocalOutcome::Completed { .. }) => Ok(TransferResult::Completed),
        Ok(LocalOutcome::Skipped) => Ok(TransferResult::Skipped),
        Ok(LocalOutcome::Canceled) => Ok(TransferResult::Canceled),
        Err(failure) => Err(failure.into_error()),
    }
}

#[cfg(test)]
#[path = "safe_file_tests.rs"]
mod tests;
