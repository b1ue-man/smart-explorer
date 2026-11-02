//! Large read-only files need no whole-file working copy on the local disk.
//!
//! DRV-M8: the Android documents provider reads every read-only handle by
//! ranges (`RangeReadPolicy::ReadOnlyHandles`), also on a writable root, and
//! opens an engine at the root of a saved connection with its own name rules
//! (`open_rooted`).
use super::engine::{
    baseline_from_meta, read_lock, require_regular, MountEngine, OpenHandle, OpenHandleKind,
};
use super::types::{BackendRoot, MountMode, MountRuntimeConfig};
use crate::vfs::BackendHandle;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::Path;

/// Which handles read large files by ranges instead of a whole-file copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeReadPolicy {
    /// Read-only drives only (every drive before DRV-M8).
    #[default]
    ReadOnlyMount,
    /// Every handle opened without write access (Android, DRV-M8). A dirty
    /// or conflicted local copy still wins over the remote.
    ReadOnlyHandles,
}

impl RangeReadPolicy {
    pub fn is_default(&self) -> bool {
        *self == Self::ReadOnlyMount
    }
}

impl MountEngine {
    /// An engine at `root` of `backend` (backend paths are absolute), with
    /// the plain spool, checking the root and every ancestor on the remote.
    /// The Android documents provider opens saved connections with it.
    pub fn open_rooted(
        config: MountRuntimeConfig,
        root: BackendRoot,
        backend: BackendHandle,
        spool_root: &Path,
    ) -> io::Result<Self> {
        Self::open_at_root(config, root, backend, spool_root, true, true)
    }

    pub(super) fn uses_range_reads(&self, size: u64) -> bool {
        // RW handles retain the established pinned whole-file snapshot, including
        // paging writes and old handles surviving a delete-sharing replacement;
        // `read_range` never takes a writable handle.
        let policy = self.config.mode == MountMode::ReadOnly
            || self.config.range_reads == RangeReadPolicy::ReadOnlyHandles;
        policy && size > self.config.cache.retained_bytes().max(1024 * 1024)
    }

    pub(super) fn read_range(
        &self,
        opened: &OpenHandle,
        offset: u64,
        output: &mut [u8],
    ) -> io::Result<Option<usize>> {
        let OpenHandleKind::Metadata {
            callback_path,
            meta,
        } = &opened.kind
        else {
            return Ok(None);
        };
        if opened.writable || !self.uses_range_reads(meta.size) {
            return Ok(None);
        }
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        if self.entry_for_path(path.backend())?.is_some() {
            // A recovered dirty/conflicted copy always takes precedence over
            // the remote, including an absent recovery payload.
            return Ok(None);
        }
        // The identity taken at open bounds every read: the stat after the
        // bytes proves they belong to it (a change between two reads is
        // caught by the later read's check), so no stat precedes the read.
        let expected = baseline_from_meta(meta);
        let length = meta.size.saturating_sub(offset).min(output.len() as u64) as usize;
        if length == 0 {
            let current = self.backend.stat(path.backend())?;
            require_regular(&current)?;
            if baseline_from_meta(&current) != expected {
                return Err(changed());
            }
            return Ok(Some(0));
        }
        let Some(mut reader) =
            self.backend
                .open_read_at(path.backend(), meta.id.as_deref(), offset)?
        else {
            // Preserve support for backends that cannot address a byte offset.
            return Ok(None);
        };
        // read_exact retries Interrupted and rejects a premature end, so a
        // short network read cannot become false EOF in the Windows callback.
        let read = reader.read_exact(&mut output[..length]);
        drop(reader); // Cancel unread frames and release the request gate before stat.
        if let Err(error) = read {
            // A file that shrank or was replaced ends early: report the change.
            return match self.backend.stat(path.backend()) {
                Ok(after) if baseline_from_meta(&after) != expected => Err(changed()),
                _ => Err(error),
            };
        }
        let after = self.backend.stat(path.backend())?;
        require_regular(&after)?;
        if baseline_from_meta(&after) != expected {
            return Err(changed());
        }
        Ok(Some(length))
    }
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "remote file changed during mounted range read; reopen it before continuing",
    )
}
