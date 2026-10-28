//! A member of a local packet in two steps (R4, 2026-10-28): its private
//! stage first, its publication after the packet's one intent record, which
//! then names every member's stage and destination. A crash after the intent
//! leaves no stage the resumed job does not know (it removes them), and the
//! acknowledgement compares through the stage handle that is still open.
//! Only new copies under "skip" or "keep both" travel in packets; the stage
//! and the publication are exactly those of `transfer_local_from`.
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::publication::PublicationObserver;
use super::super::staging::{stage_copy, LocalFailure, Parallel, Staged};
use super::local_publish::{self, Publication};
use super::{LocalOutcome, LocalRequest};
use crate::types::{Conflict, CopyMode};

/// A complete private copy of one packet member, not yet published.
pub(crate) struct PacketStage {
    staged: Staged,
    requested: PathBuf,
    start: PathBuf,
    conflict: Conflict,
    original: PathBuf,
}

/// What staging one member came to.
pub(crate) enum PacketStaging {
    Staged(PacketStage),
    Canceled,
}

/// Copies `request.source` into a new private stage next to `start` (the
/// member's first name, `packet_start`); nothing is published.
pub(crate) fn stage_local_from(
    request: &LocalRequest<'_>,
    start: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<PacketStaging, LocalFailure> {
    if request.mode != CopyMode::Copy || request.conflict == Conflict::Overwrite {
        return Err(LocalFailure::Target(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only new copies under skip or keep both are staged for a packet",
        )));
    }
    let source = request.source;
    let metadata = crate::local_access::symlink_metadata(source).map_err(LocalFailure::Source)?;
    if crate::local_access::metadata_is_link_like(source, &metadata) || !metadata.is_file() {
        return Err(LocalFailure::Source(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "source is not a regular, non-link file: {}",
                source.display()
            ),
        )));
    }
    if request.cancel.load(Ordering::Acquire) {
        return Ok(PacketStaging::Canceled);
    }
    let parallel = Parallel {
        threads: request.parallel.max(1),
        probe: request.probe,
    };
    // A new copy: no per-file disk sync, the destination's mode is not kept.
    match stage_copy(
        source,
        start,
        request.cancel,
        false,
        false,
        &parallel,
        progress,
    )? {
        Some(staged) => Ok(PacketStaging::Staged(PacketStage {
            staged,
            requested: request.target.to_path_buf(),
            start: start.to_path_buf(),
            conflict: request.conflict,
            original: source.to_path_buf(),
        })),
        None => Ok(PacketStaging::Canceled),
    }
}

impl PacketStage {
    /// The stage's path, for the intent's evidence.
    pub(crate) fn path(&self) -> &Path {
        &self.staged.temp
    }

    /// The stage's open handle, for the intent's evidence.
    pub(crate) fn handle(&self) -> &File {
        &self.staged.file
    }

    /// Removes the stage without publishing it.
    pub(crate) fn discard(self) -> Result<(), LocalFailure> {
        self.staged.discard().map_err(LocalFailure::Publish)
    }

    /// Publishes the stage without replacing anything: at its first name,
    /// numbered under "keep both" when that is taken meanwhile, not at all
    /// under "skip"; `observer` sees every attempt.
    pub(crate) fn publish(
        self,
        cancel: &AtomicBool,
        observer: &dyn PublicationObserver,
    ) -> Result<LocalOutcome, LocalFailure> {
        let mut races = 0usize;
        let mut quarantine = None;
        local_publish::publish(
            Publication {
                staged: self.staged,
                requested: self.requested,
                target: self.start,
                conflict: self.conflict,
                durable: false,
                cancel,
                original: &self.original,
            },
            &mut races,
            &mut quarantine,
            Some(observer),
        )
    }
}
