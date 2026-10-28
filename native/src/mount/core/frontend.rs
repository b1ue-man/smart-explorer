//! The frontend boundary of the mount engine (DRV-M2, contract
//! `MountFilesystem`).
//!
//! Dokany (Windows), the FUSE host (Linux, DRV-M4) and the Android documents
//! provider (DRV-M8) reach mounted data only through this trait. Paths are
//! callback paths in the volume's dialect (`VolumeFacts::dialect`); handles
//! and delete tokens are only valid on the implementation that issued them.
//! `MountEngine` implements every method by delegation
//! (`frontend_engine.rs`); the device tree (DRV-M6) implements it over one
//! engine per shared folder.
use std::io;
use std::sync::Arc;

use crate::vfs::VfsMeta;

use super::metadata_cache::MetadataChange;
use super::path::ProjectedPath;
use super::path_dialect::PathDialect;
use super::types::{
    BackendRoot, DeleteToken, EntryCondition, FlushOutcome, HandleId, MountId, NamespaceOutcome,
    OpenFileOptions, RenameOutcome,
};
use super::upload_queue::MountUploadStatus;

/// Asked between bounded steps of background work; `true` ends the work.
pub type StopProbe<'a> = &'a (dyn Fn() -> bool + Sync);

/// What a frontend needs to know about the volume it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeFacts {
    pub id: MountId,
    /// Exact backend root; change paths are absolute paths below it.
    pub root: BackendRoot,
    pub dialect: PathDialect,
    /// Names that differ only in letter case are different objects.
    pub case_sensitive: bool,
    pub read_only: bool,
}

/// Progress of one bounded background metadata expansion step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreloadProgress {
    pub loaded: usize,
    /// More expansion work is selectable right away.
    pub pending: bool,
}

/// Time and Unix mode a POSIX frontend set on a file with pending content
/// (`utimensat`, `chmod`); applied at its next publication through
/// `finish_stage` (decision C2, DRV-M4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingAttributes {
    pub mtime_ms: Option<i64>,
    pub mode: Option<u32>,
}

/// What a locking vault still finds on the drive after `prepare_lock`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockReadiness {
    /// Changes, deletes and namespace intents that are still unpublished.
    pub pending: usize,
    /// Why publishing stopped, when it failed.
    pub detail: Option<String>,
}

pub trait MountFilesystem: Send + Sync {
    /// Cached provider metadata only: implementations must not query the network here.
    fn capacity_snapshot(&self) -> crate::vfs::CapacitySnapshot {
        let facts = self.volume_facts();
        let mut value = crate::vfs::CapacitySnapshot::unknown(
            format!("mount:{}", facts.id.as_str()),
            facts.root.as_str(),
        );
        value.read_only = facts.read_only;
        value
    }
    fn cache_capacity_budget(&self, available_bytes: u64) -> u64 {
        available_bytes.saturating_sub(super::cache_space::CACHE_RESERVE_BYTES)
    }
    fn volume_facts(&self) -> VolumeFacts;
    /// The backend path of a callback path (same rules as every operation).
    fn project(&self, callback_path: &str) -> io::Result<ProjectedPath>;

    fn open_file(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId>;
    /// Opens a regular file without fetching its content; the first data
    /// operation fetches it.
    fn open_metadata_file(
        &self,
        callback_path: &str,
        metadata: VfsMeta,
        writable: bool,
    ) -> io::Result<HandleId>;
    fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize>;
    fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize>;
    fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize>;
    fn len(&self, handle: HandleId) -> io::Result<u64>;
    fn truncate(&self, handle: HandleId, length: u64) -> io::Result<()>;
    fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome>;
    /// The save of a closing handle (FUSE `flush`, Dokany `Cleanup`). An
    /// engine with write-back returns once the file is durable in its local
    /// cache and uploads it in the background (G4 ML1); `fsync` and
    /// `FlushFileBuffers` keep calling `flush`.
    fn flush_on_close(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        self.flush(handle)
    }
    fn close(&self, handle: HandleId) -> io::Result<()>;
    fn set_pending_attributes(
        &self,
        handle: HandleId,
        attributes: PendingAttributes,
    ) -> io::Result<()>;

    fn stat(&self, callback_path: &str) -> io::Result<VfsMeta>;
    /// Metadata from the bounded cache where it is fresh enough.
    fn stat_cached(&self, callback_path: &str) -> io::Result<VfsMeta>;
    fn stat_handle(&self, handle: HandleId) -> io::Result<VfsMeta>;
    fn list_dir(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>>;
    fn list_dir_cached(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>>;

    fn mkdir(&self, callback_path: &str) -> io::Result<NamespaceOutcome>;
    fn rename(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
    ) -> io::Result<RenameOutcome>;
    /// `rename` while the destination name is still open through another
    /// handle that allows deletion (Windows `FILE_SHARE_DELETE`).
    fn rename_with_shared_destination(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
        shared_destination_is_open: bool,
    ) -> io::Result<RenameOutcome>;
    fn flush_path(&self, callback_path: &str) -> io::Result<FlushOutcome>;
    fn begin_delete(&self, callback_path: &str, is_directory: bool) -> io::Result<DeleteToken>;
    fn cancel_delete(&self, token: DeleteToken) -> io::Result<()>;
    fn commit_delete(&self, token: DeleteToken) -> io::Result<()>;
    fn delete(&self, callback_path: &str, is_directory: bool) -> io::Result<()>;
    fn pending_deletes(&self) -> io::Result<Vec<DeleteToken>>;

    fn retry_pending_changes(&self) -> io::Result<()>;
    fn dirty_entries(&self) -> io::Result<Vec<(String, EntryCondition)>>;
    fn maintain_cache(&self) -> io::Result<()>;
    fn evict_clean(&self, callback_path: &str) -> io::Result<bool>;
    /// The root snapshot the first enumeration needs.
    fn preload_metadata(&self) -> io::Result<()>;
    /// One bounded background expansion step of the configured depth.
    fn preload_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<PreloadProgress>;
    /// One bounded refresh round of cached directories.
    fn refresh_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<()>;
    fn refresh_metadata(&self) -> io::Result<()> {
        self.refresh_metadata_while(&|| false)
    }
    /// Remote changes found by refreshes; paths are backend paths.
    fn drain_metadata_changes(&self, limit: usize) -> io::Result<Vec<MetadataChange>>;
    /// Drops cached content of a backend path a remote change touched.
    fn invalidate_content(&self, backend_path: &str, recursive: bool);
    /// Publishes pending changes and removes clean cached content before a
    /// vault locks (S2-V4, S2-V10).
    fn prepare_lock(&self) -> io::Result<LockReadiness>;
    /// Background uploads still pending (G4 ML1); `None` without write-back.
    fn upload_status(&self) -> Option<MountUploadStatus> {
        None
    }
    /// Conflicts that background uploads found since the last call.
    fn take_upload_conflicts(&self) -> Vec<(String, EntryCondition)> {
        Vec::new()
    }
    /// At unmount: tries every queued upload once more and waits for the
    /// running ones; what fails stays in the journal.
    fn finish_write_back(&self) {}
}
