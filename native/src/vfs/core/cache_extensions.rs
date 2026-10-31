//! The browsing cache passes every optional extension to the live backend:
//! listings and walks bypass the cache, writes and trash moves invalidate it.
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::super::extension_calls as calls;
use super::super::{
    BackendExtensions, ChangeNotice, ChangeSignalMode, ChangeSubscription, HashWalkItem,
    HashWalkRequest, RecursiveDeleteFailure, RecursiveDeleteProgress, RecursiveDeleteReport,
    RecycleExpectation, RecycleOutcome, StageFinish, StageFinished, TargetLimits, VfsListing,
    VfsMeta, VfsResult, VolumeIdentity,
};
use super::CachingBackend;

impl BackendExtensions for CachingBackend {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        crate::vfs::directory_reservation_supported(&*self.inner, root)
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        let result = crate::vfs::create_dir_exclusive(&*self.inner, path);
        // A lost reply can still have created the directory.
        self.invalidate(path);
        result
    }
    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        crate::vfs::directory_identity(&*self.inner, path)
    }
    fn rename_reserved_directory(&self, source: &str, destination: &str) -> VfsResult<()> {
        let result = crate::vfs::rename_reserved_directory(&*self.inner, source, destination);
        self.invalidate(source);
        self.invalidate(destination);
        result
    }
    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        crate::vfs::file_identity(&*self.inner, path)
    }

    fn capacity(&self, root: &str) -> VfsResult<crate::vfs::CapacitySnapshot> {
        crate::vfs::capacity(&*self.inner, root)
    }

    fn version_archive_policy(&self) -> crate::vfs::VersionArchivePolicy {
        calls::version_archive_policy(&*self.inner)
    }

    fn previous_state_identities(&self) -> VfsResult<Vec<String>> {
        calls::previous_state_identities(&*self.inner)
    }

    fn sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String> {
        calls::sync_child_path(&*self.inner, parent, literal_name)
    }

    fn sync_stat(&self, path: &str) -> VfsResult<VfsMeta> {
        calls::sync_stat(&*self.inner, path)
    }

    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<bool> {
        let result = calls::replace_staged_reversible(&*self.inner, staged, destination, retained);
        self.invalidate(staged);
        self.invalidate(destination);
        self.invalidate(retained);
        result
    }

    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        calls::list_dir_tolerant(&*self.inner, path)
    }

    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        calls::open_read_regular(&*self.inner, path, id)
    }

    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.invalidating_writer(path, || {
            calls::open_write_copy_stage_timed(&*self.inner, path, size, mtime_ms)
        })
    }

    fn creates_stages_exclusively(&self) -> bool {
        crate::vfs::creates_stages_exclusively(&*self.inner)
    }

    fn stage_proven(&self, stage: &str) -> bool {
        crate::vfs::stage_proven(&*self.inner, stage)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let result = calls::finish_stage(&*self.inner, stage, finish);
        self.invalidate(stage);
        result
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        calls::sync_filesystem(&*self.inner, root)
    }

    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        calls::confirm_namespace(&*self.inner, parent)
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        calls::target_limits(&*self.inner, root)
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        calls::unix_mode(&*self.inner, path)
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        calls::volume_identity(&*self.inner, root)
    }

    fn supports_duplicate_search(&self, root: &str) -> VfsResult<bool> {
        calls::supports_duplicate_search(&*self.inner, root)
    }

    fn find_duplicates(
        &self,
        root: &str,
        min_bytes: u64,
        progress: &crate::analytics::ReclaimProgress,
    ) -> VfsResult<Option<crate::analytics::DuplicateReport>> {
        calls::find_duplicates(&*self.inner, root, min_bytes, progress)
    }

    fn supports_hash_walk(&self, root: &str) -> VfsResult<bool> {
        calls::supports_hash_walk(&*self.inner, root)
    }

    fn hash_walk(
        &self,
        root: &str,
        request: HashWalkRequest,
        tx: crossbeam_channel::Sender<HashWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        calls::hash_walk(&*self.inner, root, request, tx, cancel)
    }

    fn content_digest(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<crate::vfs::ContentDigest>> {
        crate::vfs::content_digest(&*self.inner, path, cancel)
    }

    fn supports_recycle(&self, path: &str) -> VfsResult<bool> {
        calls::supports_recycle(&*self.inner, path)
    }

    fn recycle(&self, path: &str, expected: &RecycleExpectation) -> VfsResult<RecycleOutcome> {
        let result = calls::recycle(&*self.inner, path, expected);
        self.invalidate(path);
        result
    }

    fn supports_remove_tree(&self, path: &str) -> VfsResult<bool> {
        calls::supports_remove_tree(&*self.inner, path)
    }

    fn remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        let result = calls::remove_tree(&*self.inner, path, cancel, progress);
        // Also after a cancel or failure: part of the subtree may be gone.
        self.invalidate_prefix(path);
        result
    }

    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        calls::change_signal_mode(&*self.inner, root)
    }

    fn change_signal(
        &self,
        root: &str,
        poll_interval: Duration,
        tx: crossbeam_channel::Sender<ChangeNotice>,
    ) -> VfsResult<Option<ChangeSubscription>> {
        calls::change_signal(&*self.inner, root, poll_interval, tx)
    }

    fn host_batch_limits(&self) -> Option<crate::vfs::BatchLimits> {
        crate::vfs::host_batch_limits(&*self.inner)
    }

    fn single_put(&self) -> bool {
        crate::vfs::single_put(&*self.inner)
    }

    fn batch_put_times(&self) -> bool {
        crate::vfs::batch_put_times(&*self.inner)
    }

    /// A published file lands under `path`; the cache forgets it before and
    /// after, and once more when the writer is flushed or dropped.
    fn open_write_absent(&self, path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>> {
        self.invalidate(path);
        let writer = crate::vfs::open_write_absent(&*self.inner, path, size);
        self.invalidate(path);
        Ok(writer?.map(|writer| self.wrap_writer(writer, path)))
    }

    fn server_copy_batch(&self, items: &[(String, String, u64)]) -> Vec<io::Result<Option<u64>>> {
        let results = crate::vfs::server_copy_batch(&*self.inner, items);
        // Also after a failure: a stage may exist.
        for (_, stage, _) in items {
            self.invalidate(stage);
        }
        results
    }

    fn transfer_scope(&self) -> Option<Box<dyn std::any::Any + Send>> {
        crate::vfs::transfer_scope(&*self.inner)
    }

    fn transfer_evidence(&self, path: &str) -> bool {
        crate::vfs::transfer_evidence(&*self.inner, path)
    }

    fn put_batch_evidence(
        &self,
        entries: &[crate::vfs::BatchPut],
        data: &mut dyn Read,
        prepare: &mut crate::vfs::PreparePut<'_>,
    ) -> VfsResult<Vec<crate::vfs::EvidencePutOutcome>> {
        let result = crate::vfs::put_batch_evidence(&*self.inner, entries, data, prepare);
        for entry in entries {
            self.invalidate(&entry.path);
        }
        if let Ok(outcomes) = &result {
            for outcome in outcomes {
                if let crate::vfs::EvidencePutOutcome::Published { path, .. } = outcome {
                    self.invalidate(path);
                }
            }
        }
        result
    }

    fn promote_copy_stage_evidence(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<Option<crate::vfs::FileEvidence>> {
        let result = crate::vfs::promote_copy_stage_evidence(&*self.inner, staged, destination);
        self.invalidate(staged);
        self.invalidate(destination);
        result
    }

    fn open_read_evidence(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> VfsResult<Option<(Box<dyn Read + Send>, crate::vfs::FileEvidence)>> {
        crate::vfs::open_read_evidence(&*self.inner, path, id)
    }

    fn take_stage_evidence(&self, stage: &str) -> Option<crate::vfs::StageEvidence> {
        crate::vfs::take_stage_evidence(&*self.inner, stage)
    }

    fn supports_create_dirs(&self, root: &str) -> bool {
        crate::vfs::supports_create_dirs(&*self.inner, root)
    }

    fn create_dirs(&self, paths: &[String]) -> VfsResult<Vec<std::io::Result<bool>>> {
        let result = crate::vfs::create_dirs(&*self.inner, paths);
        // A lost reply can still have created some of them.
        for path in paths {
            self.invalidate_ancestors(path);
        }
        result
    }
}
