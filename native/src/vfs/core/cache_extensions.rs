//! The browsing cache passes every optional extension to the live backend:
//! listings and walks bypass the cache, writes and trash moves invalidate it.
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::super::extension_calls as calls;
use super::super::{
    BackendExtensions, ChangeNotice, ChangeSignalMode, ChangeSubscription, HashWalkItem,
    HashWalkRequest, RecycleExpectation, RecycleOutcome, StageFinish, StageFinished, TargetLimits,
    VfsListing, VfsResult, VolumeIdentity,
};
use super::CachingBackend;

impl BackendExtensions for CachingBackend {
    fn previous_state_identities(&self) -> VfsResult<Vec<String>> {
        calls::previous_state_identities(&*self.inner)
    }

    fn sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String> {
        calls::sync_child_path(&*self.inner, parent, literal_name)
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

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let result = calls::finish_stage(&*self.inner, stage, finish);
        self.invalidate(stage);
        result
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        calls::sync_filesystem(&*self.inner, root)
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

    fn supports_recycle(&self, path: &str) -> VfsResult<bool> {
        calls::supports_recycle(&*self.inner, path)
    }

    fn recycle(&self, path: &str, expected: &RecycleExpectation) -> VfsResult<RecycleOutcome> {
        let result = calls::recycle(&*self.inner, path, expected);
        self.invalidate(path);
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
}
