//! Optional backend extensions beyond the core `Backend` interface. A backend
//! opts in by implementing `BackendExtensions` and returning itself from
//! `Backend::extensions`; callers use the free functions of
//! `extension_calls.rs`, which supply the fallbacks for everyone else.
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::extension_types::{
    ChangeNotice, ChangeSignalMode, ChangeSubscription, HashWalkItem, HashWalkRequest,
    RecycleExpectation, RecycleOutcome, StageFinish, StageFinished, TargetLimits, VfsListing,
};
use super::{Backend, VfsResult, VolumeIdentity};

/// Every method has a "not supported" default, so an implementation names
/// only what its protocol or host can do. Wrappers that rewrite paths or
/// guard access (export roots, mounts) must not hand out the extensions of
/// the backend they wrap unchanged.
pub trait BackendExtensions: Backend {
    /// Build a provider path for one literal sync name. Persisted locators
    /// and the parent keep their existing encoding; providers encode only
    /// the new literal component. Never decode or canonicalize the parent.
    fn sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String> {
        super::extension_calls::default_sync_child_path(parent, literal_name)
    }

    /// Publish a stage on protocols lacking an atomic replacing rename.
    /// The caller durably journals the exact absent sibling `retained`
    /// before this call. `Ok(false)` means unsupported without mutation;
    /// `Ok(true)` leaves the old original at `retained`. An error restores
    /// without replacing or preserves the original at that same known path.
    /// Implementations never delete the retained original and must not claim
    /// atomic namespace replacement for this reversible sequence.
    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<bool> {
        let _ = (staged, destination, retained);
        Ok(false)
    }

    /// `list_dir` that keeps going past entries it cannot list.
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        self.list_dir(path).map(VfsListing::complete)
    }

    /// Open an entry a listing reported as a regular file: a link, folder or
    /// special file at `path` is refused (`InvalidInput`), never followed or
    /// waited on. Remote protocols keep their server's `open_read` semantics.
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.open_read_id(path, id)
    }

    /// `open_write_copy_stage_sized` plus the source time, for providers that
    /// can store it only while uploading (WebDAV `X-OC-Mtime`, Drive
    /// `modifiedTime`); `finish_stage` reports whether it took effect.
    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        let _ = mtime_ms;
        self.open_write_copy_stage_sized(path, size)
    }

    /// Give a complete, closed, unpublished stage its source time and the
    /// requested durability before `promote_*` publishes it. Failing to store
    /// the time is no error (`mtime_applied: false`); failing to flush is.
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let _ = (stage, finish);
        Ok(StageFinished::default())
    }

    /// Make every stage that was finished with `StageDurability::Deferred`
    /// and published below `root` durable, including the renames that
    /// published it (Linux `syncfs`). `Ok(false)` = no such guarantee here.
    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        let _ = root;
        Ok(false)
    }

    /// What the filesystem or protocol below `root` can store (names, file
    /// size, time resolution); unknown parts stay `None`/`Unknown`.
    fn target_limits(&self, root: &str) -> TargetLimits {
        let _ = root;
        TargetLimits::default()
    }

    /// Unix permission bits of `path` where this backend keeps Unix modes
    /// (local Linux/Android, SFTP); `None` elsewhere.
    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        let _ = path;
        Ok(None)
    }

    /// Filesystem identity of `root` independent of its mount point (the
    /// replica fallback where no marker can be written). `Ok(None)` = not
    /// determinable, which means "unknown", never "another volume".
    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        let _ = root;
        Ok(None)
    }

    /// Whether `find_duplicates` runs on the storing host for `root`.
    fn supports_duplicate_search(&self, root: &str) -> VfsResult<bool> {
        let _ = root;
        Ok(false)
    }

    /// Duplicate search on the device that stores `root` (files of at least
    /// `min_bytes`); counters, phase and cancel go through `progress`, paths
    /// in the report are paths of this backend. `Ok(None)` = unsupported.
    fn find_duplicates(
        &self,
        root: &str,
        min_bytes: u64,
        progress: &crate::analytics::ReclaimProgress,
    ) -> VfsResult<Option<crate::analytics::DuplicateReport>> {
        let _ = (root, min_bytes, progress);
        Ok(None)
    }

    /// Whether `hash_walk` runs on the storing host for `root`.
    fn supports_hash_walk(&self, root: &str) -> VfsResult<bool> {
        let _ = root;
        Ok(false)
    }

    /// Walk `root` on the storing host and stream every folder, regular file
    /// (with digest when requested) and omission into `tx`. `Ok(false)` only
    /// for "unsupported" before any item; a later failure (transport, host,
    /// cancel) is an error, so a partial walk is never taken as complete.
    fn hash_walk(
        &self,
        root: &str,
        request: HashWalkRequest,
        tx: crossbeam_channel::Sender<HashWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        let _ = (root, request, tx, cancel);
        Ok(false)
    }

    /// Whether `recycle` can move `path` into its device's trash.
    fn supports_recycle(&self, path: &str) -> VfsResult<bool> {
        let _ = path;
        Ok(false)
    }

    /// Move `path` into the trash of the device that stores it after checking
    /// it still matches `expected`; `Unsupported` where there is none.
    fn recycle(&self, path: &str, expected: &RecycleExpectation) -> VfsResult<RecycleOutcome> {
        let _ = (path, expected);
        super::meta::unsupported("recycling on the storing device is not supported")
    }

    /// How `change_signal` learns about changes below `root`.
    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        let _ = root;
        Ok(None)
    }

    /// Subscribe to changes below `root`: notices go to `tx` until the
    /// returned subscription is dropped; poll-mode sources ask once per
    /// `poll_interval`. `Ok(None)` = unsupported (the caller polls itself).
    fn change_signal(
        &self,
        root: &str,
        poll_interval: Duration,
        tx: crossbeam_channel::Sender<ChangeNotice>,
    ) -> VfsResult<Option<ChangeSubscription>> {
        let _ = (root, poll_interval, tx);
        Ok(None)
    }
}
