//! Optional operations of the Share identity behind an AgentBackend. The
//! identity retains its PeerOpenTarget and reconnects that exact target;
//! a relative path is never resolved on the GUI's filesystem.
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::vfs::{self as vfs, BackendExtensions, ChangeNotice, ChangeSignalMode,
    ChangeSubscription, HashWalkItem, HashWalkRequest, RecycleExpectation, RecycleOutcome,
    StageFinish, StageFinished, TargetLimits, VfsListing, VfsResult, VolumeIdentity};
use super::ipc_client::UnavailableBackend;

impl BackendExtensions for UnavailableBackend {
    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        vfs::list_dir_tolerant(&*self.live_backend()?, path)
    }

    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        vfs::open_read_regular(&*self.live_backend()?, path, id)
    }

    fn open_write_copy_stage_timed(&self, path: &str, size: u64, mtime_ms: i64)
        -> VfsResult<Box<dyn Write + Send>> {
        vfs::open_write_copy_stage_timed(&*self.live_backend()?, path, size, mtime_ms)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        vfs::finish_stage(&*self.live_backend()?, stage, finish)
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        vfs::sync_filesystem(&*self.live_backend()?, root)
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        self.live_backend().map(|backend| vfs::target_limits(&*backend, root)).unwrap_or_default()
    }

    fn unix_mode(&self, _path: &str) -> VfsResult<Option<u32>> {
        // The worker protocol offers no Unix-mode/volume-identity query yet.
        // Unknown is safer than discovering these facts on the client device.
        Ok(None)
    }

    fn volume_identity(&self, _root: &str) -> VfsResult<Option<VolumeIdentity>> {
        Ok(None)
    }

    fn supports_duplicate_search(&self, root: &str) -> VfsResult<bool> {
        vfs::supports_duplicate_search(&*self.live_backend()?, root)
    }

    fn find_duplicates(&self, root: &str, min_bytes: u64,
        progress: &crate::analytics::ReclaimProgress)
        -> VfsResult<Option<crate::analytics::DuplicateReport>> {
        vfs::find_duplicates(&*self.live_backend()?, root, min_bytes, progress)
    }

    fn supports_hash_walk(&self, root: &str) -> VfsResult<bool> {
        vfs::supports_hash_walk(&*self.live_backend()?, root)
    }

    fn hash_walk(&self, root: &str, request: HashWalkRequest,
        tx: crossbeam_channel::Sender<HashWalkItem>, cancel: &AtomicBool) -> VfsResult<bool> {
        vfs::hash_walk(&*self.live_backend()?, root, request, tx, cancel)
    }

    fn supports_recycle(&self, path: &str) -> VfsResult<bool> {
        vfs::supports_recycle(&*self.live_backend()?, path)
    }

    fn recycle(&self, path: &str, expected: &RecycleExpectation) -> VfsResult<RecycleOutcome> {
        vfs::recycle(&*self.live_backend()?, path, expected)
    }

    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        vfs::change_signal_mode(&*self.live_backend()?, root)
    }

    fn change_signal(&self, root: &str, poll_interval: Duration,
        tx: crossbeam_channel::Sender<ChangeNotice>) -> VfsResult<Option<ChangeSubscription>> {
        vfs::change_signal(&*self.live_backend()?, root, poll_interval, tx)
    }
}
