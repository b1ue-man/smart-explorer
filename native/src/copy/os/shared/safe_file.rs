//! One local file copied or moved into a prepared folder, in three phases:
//! the first name (conflict policy), the private stage (kernel copy, the
//! source observed), and the publication (`local_publish`: no-replace rename
//! under the policy, each attempt behind the observer's durable intent). A
//! move first relocates its source to an unpredictable quarantine. New copies
//! skip the per-file disk sync; moves and replacements keep it, and since
//! M15 (2026-10-28) they copy with the same range threads as new copies.
use crate::types::{Conflict, CopyMode};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::platform;
#[cfg(test)]
use super::move_guard::{quarantine_source, restore_quarantine, source_snapshot_path};
use super::move_guard::{
    quarantine_source_observed, restore_after_error, restore_quarantine_if_any,
};
#[cfg(test)]
use super::path_guard::prepare_target_parent;
use super::publication::PublicationObserver;
use super::staging::{stage_copy, LocalFailure, Parallel};
use crate::transfer::update_rule::Replacing;

#[path = "direct_move.rs"]
mod direct_move;
#[path = "local_publish.rs"]
mod local_publish;
pub(crate) use local_publish::{packet_start, still_decided as target_still_decided};
#[path = "packet_stage.rs"]
mod packet_stage;
pub(crate) use packet_stage::{stage_local_from, PacketStage, PacketStaging};

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
    /// Durable copies (moves across volumes, replacements) use them too
    /// (M15): their stage is still synced as a whole before publication.
    pub parallel: usize,
    /// Set (with `parallel` > 1) for a drive not known to be flash: the copy
    /// measures whether ranges help and reports the verdict here; the rest of
    /// the file is copied accordingly.
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
    transfer_local_inner(request, Start::Look, progress, None, None)
}

/// The same local transfer with durable receipt hooks around publication.
pub(crate) fn transfer_local_observed(
    request: &LocalRequest<'_>,
    progress: &mut dyn FnMut(u64),
    observer: &dyn PublicationObserver,
) -> Result<LocalOutcome, LocalFailure> {
    transfer_local_inner(request, Start::Look, progress, Some(observer), None)
}

/// `transfer_local_observed` for a replacement the file rule decided
/// (`request.conflict` is `Overwrite`): right before the stage replaces the
/// target, the target's size and time are compared with what the decision
/// saw (`decided`). A target saved meanwhile is not replaced; the copy is
/// published beside it under the next free name instead (the returned
/// `target` then differs from `request.target`), so nothing is lost.
pub(crate) fn transfer_local_replacing(
    request: &LocalRequest<'_>,
    decided: &Replacing,
    progress: &mut dyn FnMut(u64),
    observer: &dyn PublicationObserver,
) -> Result<LocalOutcome, LocalFailure> {
    transfer_local_inner(
        request,
        Start::Look,
        progress,
        Some(observer),
        Some(decided),
    )
}

/// `transfer_local_observed` for a new copy under "skip" into a folder this
/// job created (A7, 2026-10-29): nothing of anyone else can be inside, so
/// the destination is not looked at first. A name taken meanwhile still
/// fails the no-replace publication, which then skips the file, as the
/// look would have.
pub(crate) fn transfer_local_fresh(
    request: &LocalRequest<'_>,
    progress: &mut dyn FnMut(u64),
    observer: &dyn PublicationObserver,
) -> Result<LocalOutcome, LocalFailure> {
    let fresh = request.mode == CopyMode::Copy && request.conflict == Conflict::Skip;
    let start = if fresh { Start::Fresh } else { Start::Look };
    transfer_local_inner(request, start, progress, Some(observer), None)
}

/// Where the first name of a transfer comes from.
enum Start<'p> {
    /// The conflict policy looks at the destination (`select_initial_target`).
    Look,
    /// The requested name, unlooked at: its folder is new (`transfer_local_fresh`).
    Fresh,
    /// Chosen before by the caller (`transfer_local_from`).
    At(&'p Path),
}

/// A member of a local packet (M3): a new copy under "skip" or "keep both"
/// whose first name `start` was chosen by `packet_start` before the packet's
/// intent; "keep both" numbers from `request.target` on any later conflict.
/// Stage and publication are those of `transfer_local_observed`.
pub(crate) fn transfer_local_from(
    request: &LocalRequest<'_>,
    start: &Path,
    progress: &mut dyn FnMut(u64),
    observer: &dyn PublicationObserver,
) -> Result<LocalOutcome, LocalFailure> {
    if request.mode != CopyMode::Copy || request.conflict == Conflict::Overwrite {
        return Err(LocalFailure::Target(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only new copies under skip or keep both start at a chosen name",
        )));
    }
    transfer_local_inner(request, Start::At(start), progress, Some(observer), None)
}

fn transfer_local_inner(
    request: &LocalRequest<'_>,
    start: Start<'_>,
    progress: &mut dyn FnMut(u64),
    observer: Option<&dyn PublicationObserver>,
    decided: Option<&Replacing>,
) -> Result<LocalOutcome, LocalFailure> {
    let src = request.source;
    // A copy's source opened once where the platform can classify the
    // handle (Windows, A7); a move's source is quarantined by name first.
    let opened = if request.mode == CopyMode::Copy {
        checked_copy_source(src)?
    } else {
        check_source(src)?;
        None
    };
    // Phase 1: the first name.
    let requested = request.target.to_path_buf();
    let mut target = match start {
        Start::At(start) => start.to_path_buf(),
        Start::Fresh => requested.clone(),
        Start::Look => match select_initial_target(src, &requested, request.conflict)? {
            Some(target) => target,
            None => return Ok(LocalOutcome::Skipped),
        },
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
            request,
            &source_path,
            &requested,
            &mut target,
            &mut races,
            &mut quarantine,
            observer,
            decided,
        )? {
            return Ok(outcome);
        }
    }

    // Phase 2: the stage. A durable copy (a move to another volume, a
    // replacement) takes the range threads too (M15, 2026-10-28): the
    // threads only decide how the bytes get into the stage, which is synced
    // as a whole (`stage_copy`) before the publication below, so "durable
    // before published" holds as with one thread.
    let durable = request.mode == CopyMode::Move || request.conflict == Conflict::Overwrite;
    let parallel = Parallel {
        threads: request.parallel.max(1),
        probe: request.probe,
    };
    #[cfg(test)]
    durable_ranges_tests::saw(parallel.threads, durable);
    let staged = match stage_copy(
        &source_path,
        opened,
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
    // Phase 3: the publication.
    local_publish::publish(
        local_publish::Publication {
            staged,
            requested,
            target,
            conflict: request.conflict,
            durable,
            cancel: request.cancel,
            original: src,
            decided,
        },
        &mut races,
        &mut quarantine,
        observer,
    )
}

/// The source of a transfer, checked to be a regular file that is no link:
/// opened already where the platform classifies the handle itself
/// (`open_copy_source`, Windows), else looked at (`None`; the stage opens it).
pub(super) fn checked_copy_source(src: &Path) -> Result<Option<std::fs::File>, LocalFailure> {
    if let Some(file) = platform::open_copy_source(src) {
        return Ok(Some(file));
    }
    check_source(src).map(|()| None)
}

/// Refuses a source that is no regular file or a link, by one look at it.
fn check_source(src: &Path) -> Result<(), LocalFailure> {
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
    Ok(())
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
    if matches!(conflict, Conflict::Rename | Conflict::Update) {
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
#[path = "abgleich_task_replace_tests.rs"]
mod abgleich_task_replace_tests;
#[cfg(test)]
#[path = "durable_ranges_tests.rs"]
mod durable_ranges_tests;
#[cfg(test)]
#[path = "safe_file_tests.rs"]
mod tests;
