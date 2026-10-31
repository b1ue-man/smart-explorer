//! The publication phase of one local file (`safe_file`): the private stage
//! is checked to be the object that was copied, the observer's durable
//! intent comes before every no-replace rename, a conflict is decided under
//! the policy ("keep both" probes numbered names, each with its own intent),
//! and a durable publication (move, replacement) is confirmed and its folders
//! synced before a quarantined move source goes away.
//!
//! A packet of new local copies (M3, 2026-10-28) decides each member's first
//! name before the packet's one intent record (`packet_start`), so a member
//! whose requested name is free is covered by that record and one that must
//! be numbered gets its own intent when it publishes.
use crate::transfer::update_rule::{Replacing, Stamp};
use crate::types::Conflict;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::super::platform;
use super::super::durability::finish_staged_commit;
use super::super::move_guard::{
    restore_after_error, restore_quarantine_if_any, source_changed_error, QuarantinedSource,
};
use super::super::publication::{self, PublicationObserver};
use super::super::staging::{LocalFailure, Staged};
use super::{metadata_if_exists, next_name, unique_path, LocalOutcome};

/// One complete private copy and where it is to be published.
pub(super) struct Publication<'r> {
    pub staged: Staged,
    /// The name the job asked for: "keep both" numbers from it.
    pub requested: PathBuf,
    /// The name tried first (the requested one, or a numbered one).
    pub target: PathBuf,
    pub conflict: Conflict,
    /// Moves and replacements: the stage was synced, the publication is
    /// confirmed and both folders are synced.
    pub durable: bool,
    pub cancel: &'r AtomicBool,
    /// The source as the job named it (the quarantine's original name).
    pub original: &'r Path,
    /// A replacement the file rule decided: what it saw of the target. The
    /// target is compared with it right before the stage replaces it; a
    /// target that changed meanwhile is kept and the copy is numbered.
    pub decided: Option<&'r Replacing>,
}

/// Whether the file at `target` still is what the rule decided to replace
/// (a regular file of the stamp's size and time); a missing target is no
/// replacement but a plain no-replace publication.
pub(crate) fn still_decided(target: &Path, decided: &Replacing) -> bool {
    match metadata_if_exists(target) {
        Ok(None) => true,
        Ok(Some(now)) => {
            now.is_file()
                && !crate::local_access::metadata_is_link_like(target, &now)
                && decided.holds(Stamp {
                    size: now.len(),
                    mtime_ms: now
                        .modified()
                        .map(crate::local_access::system_time_ms)
                        .unwrap_or(0),
                })
        }
        Err(_) => false,
    }
}

/// Publishes `publication`; `quarantine` is a move's secured source, given
/// back on every path that does not publish.
pub(super) fn publish(
    publication: Publication<'_>,
    races: &mut usize,
    quarantine: &mut Option<QuarantinedSource>,
    observer: Option<&dyn PublicationObserver>,
) -> Result<LocalOutcome, LocalFailure> {
    let Publication {
        staged,
        requested,
        mut target,
        mut conflict,
        durable,
        cancel,
        original,
        decided,
    } = publication;
    if cancel.load(Ordering::Acquire) {
        staged.discard().map_err(LocalFailure::Publish)?;
        restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
        return Ok(LocalOutcome::Canceled);
    }
    // The stage path still names the copied object: asked of the handle's
    // own name where the platform can (Windows, A7: no open), else by one
    // look at the path.
    let same = if platform::handle_has_path(&staged.file, &staged.temp) {
        Ok(true)
    } else {
        let identity = platform::file_identity(&staged.file).map_err(LocalFailure::Target);
        identity.and_then(|identity| {
            platform::path_matches_identity(&staged.temp, identity).map_err(LocalFailure::Target)
        })
    };
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
    if let Some(source) = quarantine.as_ref() {
        if staged.snapshot != source.snapshot {
            staged.discard().map_err(LocalFailure::Publish)?;
            return Err(LocalFailure::Source(restore_after_error(
                quarantine.as_ref(),
                source_changed_error(&source.path),
            )));
        }
    }

    // A replacement the file rule decided is bound to the target it saw: one
    // look right before the stage may replace it. A target that was saved
    // again meanwhile is kept, and the copy is numbered like "keep both".
    if conflict == Conflict::Overwrite {
        if let Some(decided) = decided {
            if !still_decided(&target, decided) {
                conflict = Conflict::Rename;
            }
        }
    }
    loop {
        if let Err(error) = publication::before(observer, &staged.temp, &target, &staged.file) {
            publication::preparation_failed(
                observer,
                &staged.temp,
                &target,
                &error,
                quarantine.as_ref(),
            )
            .map_err(LocalFailure::Publish)?;
            staged.discard().map_err(LocalFailure::Publish)?;
            return Err(LocalFailure::Publish(restore_after_error(
                quarantine.as_ref(),
                error,
            )));
        }
        // A pinned stage is renamed through its own handle (Windows, A7).
        let committed = platform::commit_staged_held(
            &staged.temp,
            &target,
            staged.pinned.then_some(&staged.file),
            conflict == Conflict::Overwrite,
            durable,
        );
        if let Err(error) = &committed {
            publication::commit_failed(
                observer,
                &staged.temp,
                &target,
                &staged.file,
                error,
                quarantine.as_ref(),
            )
            .map_err(LocalFailure::Publish)?;
        }
        match committed {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match conflict {
                Conflict::Skip => {
                    staged.discard().map_err(LocalFailure::Publish)?;
                    restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
                    return Ok(LocalOutcome::Skipped);
                }
                // `Update` never reaches this layer (the engine decides it per
                // file); were it to, keeping both destroys nothing.
                Conflict::Rename | Conflict::Update => match next_name(&requested, races) {
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
        crate::copy::publication_proof::confirm_handle_publication(&target, &staged.file, cancel)
            .map_err(|error| {
                let retained = quarantine.as_ref().map_or(String::new(), |source|
                    format!("; secured move source retained at {}", source.path.display()));
                LocalFailure::Publish(io::Error::new(error.kind(), format!(
                    "destination committed at {}{retained}; destination durability confirmation failed: {error}",
                    target.display(),
                )))
            })?;
    }
    publication::after(observer, &target, &staged.file, quarantine.as_ref())
        .map_err(LocalFailure::Publish)?;
    drop(staged.file);
    if durable {
        finish_staged_commit(&target, original, quarantine, platform::sync_parent)
            .map_err(LocalFailure::Publish)?;
    }
    Ok(LocalOutcome::Completed { bytes, target })
}

/// Where a member of a local packet starts (M3), decided before the
/// packet's intent: `None` = skip (the requested name exists under "skip");
/// the requested name when it is free; under "keep both" the first free
/// numbered name when it is not. One look at the destination per name,
/// as the single file's own conflict check costs. Only new copies under
/// "skip" or "keep both" travel in packets: a move secures its source with
/// an intent of its own and a replacement records what it replaces.
pub(crate) fn packet_start(
    requested: &Path,
    conflict: Conflict,
) -> Result<Option<PathBuf>, LocalFailure> {
    match conflict {
        Conflict::Skip => match metadata_if_exists(requested).map_err(LocalFailure::Target)? {
            Some(_) => Ok(None),
            None => Ok(Some(requested.to_path_buf())),
        },
        Conflict::Rename | Conflict::Update => unique_path(requested)
            .map(Some)
            .map_err(LocalFailure::Target),
        Conflict::Overwrite => Err(LocalFailure::Target(io::Error::new(
            io::ErrorKind::InvalidInput,
            "replacements are not copied in packets",
        ))),
    }
}
