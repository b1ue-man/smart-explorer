//! RV1 backend extension surface; negotiation keeps older peers on their existing paths.
use super::{
    backend::PeerBackend,
    peer_stream,
    wire::{FsRecycle, FsRequest, FsResponse, FsStageDurability, FsStageFinish, FsSyncFilesystem},
};
use crate::{
    analytics::{DuplicateReport, ReclaimProgress},
    vfs::{
        Backend, BackendExtensions, ChangeNotice, ChangeSignalMode, ChangeSubscription,
        HashWalkItem, HashWalkRequest, RecycleExpectation, RecycleOutcome, StageDurability,
        StageFinish, StageFinished, TargetLimits, VfsListing,
    },
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[path = "peer_literal_paths.rs"]
pub(super) mod literal_paths;
#[path = "peer_reversible_replace.rs"]
pub(super) mod reversible_replace;

impl BackendExtensions for PeerBackend {
    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> io::Result<bool> {
        reversible_replace::client(self, staged, destination, retained)
    }
    fn sync_child_path(&self, parent: &str, literal_name: &str) -> io::Result<String> {
        literal_paths::client(self, parent, literal_name)
    }
    fn list_dir_tolerant(&self, path: &str) -> io::Result<VfsListing> {
        super::peer_list_batch::list(self, path)
    }
    fn finish_stage(&self, stage: &str, finish: StageFinish) -> io::Result<StageFinished> {
        let ticket = self.verify_owned_stage(stage)?;
        if !peer_stream::features(self, stage)?.stage_finish_v1 {
            return Ok(StageFinished::default());
        }
        let durability = match finish.durability {
            StageDurability::NotRequired => FsStageDurability::NotRequired,
            StageDurability::Deferred => FsStageDurability::Deferred,
            StageDurability::Now => FsStageDurability::Now,
        };
        ticket.begin()?;
        match self.stage_request_once(
            FsRequest::FinishStage(FsStageFinish {
                staged: stage.into(),
                mtime_ms: finish.mtime_ms,
                mode: finish.mode,
                durability,
            }),
            "finish_stage",
        )? {
            FsResponse::StageFinished {
                mtime_applied,
                durable,
            } => {
                ticket.finished(&self.stat(stage)?)?;
                Ok(StageFinished {
                    mtime_applied,
                    durable,
                })
            }
            _ => Err(peer_stream::invalid("Unerwartete Stage-Antwort")),
        }
    }
    fn sync_filesystem(&self, root: &str) -> io::Result<bool> {
        if !peer_stream::features(self, root)?.stage_finish_v1 {
            return Ok(false);
        }
        match self.request(FsRequest::SyncFilesystem(FsSyncFilesystem {
            path: root.into(),
        }))? {
            FsResponse::Synced { durable } => Ok(durable),
            _ => Err(peer_stream::invalid("Unerwartete Haltbarkeits-Antwort")),
        }
    }
    fn target_limits(&self, root: &str) -> TargetLimits {
        match self.legacy_capabilities() {
            Ok(false) => {}
            Ok(true) | Err(_) => return TargetLimits::default(),
        }
        match self.request(FsRequest::Capabilities {
            path: root.into(),
            acquire_lease: false,
            lease_request_id: None,
        }) {
            Ok(FsResponse::Capabilities { capabilities, .. }) => capabilities.limits.into(),
            _ => TargetLimits::default(),
        }
    }
    fn supports_duplicate_search(&self, root: &str) -> io::Result<bool> {
        Ok(peer_stream::features(self, root)?.duplicate_search_v1)
    }
    fn find_duplicates(
        &self,
        root: &str,
        min_bytes: u64,
        p: &ReclaimProgress,
    ) -> io::Result<Option<DuplicateReport>> {
        super::peer_duplicates::find(self, root, min_bytes, p)
    }
    fn supports_hash_walk(&self, root: &str) -> io::Result<bool> {
        Ok(peer_stream::features(self, root)?.hash_walk_v1)
    }
    fn hash_walk(
        &self,
        root: &str,
        request: HashWalkRequest,
        tx: crossbeam_channel::Sender<HashWalkItem>,
        cancel: &AtomicBool,
    ) -> io::Result<bool> {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        super::peer_hash_walk::walk(self, root, request, tx, cancel)
    }
    fn supports_recycle(&self, path: &str) -> io::Result<bool> {
        Ok(peer_stream::features(self, path)?.remote_trash_v1)
    }
    fn recycle(&self, path: &str, expected: &RecycleExpectation) -> io::Result<RecycleOutcome> {
        if !self.supports_recycle(path)? {
            return Err(io::ErrorKind::Unsupported.into());
        }
        match self.request(FsRequest::Recycle(FsRecycle {
            path: path.into(),
            expected_size: expected.size,
            expected_sha256: expected.sha256.clone(),
        }))? {
            FsResponse::Recycle { moved: true } => Ok(RecycleOutcome::Recycled),
            FsResponse::Recycle { moved: false } => Ok(RecycleOutcome::Changed),
            _ => Err(peer_stream::invalid("Unerwartete Papierkorb-Antwort")),
        }
    }
    fn change_signal_mode(&self, root: &str) -> io::Result<Option<ChangeSignalMode>> {
        Ok(peer_stream::features(self, root)?
            .watch_v1
            .then_some(ChangeSignalMode::Push))
    }
    fn change_signal(
        &self,
        root: &str,
        _: Duration,
        tx: crossbeam_channel::Sender<ChangeNotice>,
    ) -> io::Result<Option<ChangeSubscription>> {
        super::peer_watch::subscribe(self, root, tx)
    }
}
