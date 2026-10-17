//! Delivery of remote changes to a filesystem frontend (DRV-M2), shared by
//! every platform. The pump drains the engine's metadata changes in bounded
//! batches, drops cached content they touch, maps each backend path to the
//! volume's callback path and hands it to the platform sink (Dokany
//! notifications on Windows, kernel invalidation under FUSE). A change the
//! sink does not take stays queued, so a transient failure never loses one.
use std::collections::VecDeque;
use std::io;

use crate::mount::metadata_cache::MetadataChange;
use crate::mount::{MountFilesystem, VolumeFacts};

/// Changes drained and delivered per host loop turn (value of v0.5.182).
pub(crate) const CHANGE_BATCH: usize = 128;

/// Where a frontend receives remote changes; paths are callback paths of
/// the volume's dialect.
pub(crate) trait ChangeSink {
    fn created(&mut self, callback_path: &str, is_directory: bool) -> io::Result<()>;
    fn deleted(&mut self, callback_path: &str, is_directory: bool) -> io::Result<()>;
    fn modified(&mut self, callback_path: &str) -> io::Result<()>;
}

#[derive(Default)]
pub(crate) struct ChangePump {
    pending: VecDeque<MetadataChange>,
}

impl ChangePump {
    /// Delivers at most one batch. Only the host loop calls it: a frontend
    /// must never notify from inside its own filesystem callbacks.
    pub(crate) fn deliver(
        &mut self,
        filesystem: &dyn MountFilesystem,
        sink: &mut dyn ChangeSink,
    ) -> io::Result<()> {
        if self.pending.is_empty() {
            for change in filesystem.drain_metadata_changes(CHANGE_BATCH)? {
                let recursive = matches!(
                    &change,
                    MetadataChange::Deleted {
                        is_directory: true,
                        ..
                    } | MetadataChange::Modified { .. }
                );
                filesystem.invalidate_content(change.path(), recursive);
                self.pending.push_back(change);
            }
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        let facts = filesystem.volume_facts();
        for _ in 0..CHANGE_BATCH {
            let Some(change) = self.pending.front() else {
                break;
            };
            let path = callback_path(&facts, filesystem, change.path())?;
            match change {
                MetadataChange::Created { is_directory, .. } => {
                    sink.created(&path, *is_directory)?
                }
                MetadataChange::Deleted { is_directory, .. } => {
                    sink.deleted(&path, *is_directory)?
                }
                MetadataChange::Modified { .. } => sink.modified(&path)?,
            }
            self.pending.pop_front();
        }
        Ok(())
    }
}

/// The callback path of a backend path below the volume root. A path
/// outside the root, or one the volume would project to another backend
/// path, is refused rather than guessed.
pub(crate) fn callback_path(
    facts: &VolumeFacts,
    filesystem: &dyn MountFilesystem,
    backend: &str,
) -> io::Result<String> {
    let root = facts.root.as_str();
    let suffix = backend
        .strip_prefix(root)
        .filter(|suffix| root == "/" || suffix.is_empty() || suffix.starts_with('/'))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "notification path escaped its mounted root",
            )
        })?;
    let callback = facts.dialect.callback_path(suffix.trim_start_matches('/'));
    if filesystem.project(&callback)?.backend() != backend {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "notification path is not canonical",
        ));
    }
    Ok(callback)
}
