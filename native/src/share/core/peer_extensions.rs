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
        HashWalkItem, HashWalkRequest, RecursiveDeleteFailure, RecursiveDeleteProgress,
        RecursiveDeleteReport, RecycleExpectation, RecycleOutcome, StageDurability, StageFinish,
        StageFinished, TargetLimits, VfsListing,
    },
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[path = "peer_literal_paths.rs"]
pub(super) mod literal_paths;
#[path = "peer_remove_tree.rs"]
mod remove_tree;
#[path = "peer_reversible_replace.rs"]
pub(super) mod reversible_replace;

#[path = "file_identity_rpc.rs"]
pub(in crate::share) mod file_identity;
#[path = "folder_reservation.rs"]
pub(in crate::share) mod folder_reservation;

impl BackendExtensions for PeerBackend {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        folder_reservation::supported(self, root)
    }

    fn create_dir_exclusive(&self, path: &str) -> io::Result<()> {
        folder_reservation::create(self, path)
    }
    fn directory_identity(&self, path: &str) -> io::Result<Option<String>> {
        folder_reservation::identity(self, path)
    }
    fn rename_reserved_directory(&self, source: &str, destination: &str) -> io::Result<()> {
        folder_reservation::rename(self, source, destination)
    }
    fn file_identity(&self, path: &str) -> io::Result<Option<String>> {
        file_identity::client(self, path)
    }
    fn capacity(&self, root: &str) -> io::Result<crate::vfs::CapacitySnapshot> {
        super::capacity::client(self, root)
    }
    fn version_archive_policy(&self) -> crate::vfs::VersionArchivePolicy {
        crate::vfs::VersionArchivePolicy::AppPrivate
    }

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
    fn content_digest(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> io::Result<Option<crate::vfs::ContentDigest>> {
        super::peer_content_digest::digest(self, path, cancel)
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
    fn supports_remove_tree(&self, path: &str) -> io::Result<bool> {
        Ok(peer_stream::features(self, path)?.remove_tree_v1)
    }
    fn remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        remove_tree::remove(self, path, cancel, progress)
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
    fn host_batch_limits(&self) -> Option<crate::vfs::BatchLimits> {
        self.host_batch_limits_any()
    }
    fn single_put(&self) -> bool {
        self.single_put_offered()
    }
    fn batch_put_times(&self) -> bool {
        self.batch_times_offered()
    }
    fn transfer_evidence(&self, _path: &str) -> bool {
        self.evidence_offered()
    }
    fn put_batch_evidence(
        &self,
        entries: &[crate::vfs::BatchPut],
        data: &mut dyn std::io::Read,
        prepare: &mut crate::vfs::PreparePut<'_>,
    ) -> io::Result<Vec<crate::vfs::EvidencePutOutcome>> {
        self.put_batch_evidenced(entries, data, prepare)
    }
    fn promote_copy_stage_evidence(
        &self,
        staged: &str,
        destination: &str,
    ) -> io::Result<Option<crate::vfs::FileEvidence>> {
        self.promote_evidenced(staged, destination)
    }
    fn open_read_evidence(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> io::Result<Option<(Box<dyn std::io::Read + Send>, crate::vfs::FileEvidence)>> {
        self.read_evidenced(path, id)
    }
    fn take_stage_evidence(&self, stage: &str) -> Option<crate::vfs::StageEvidence> {
        self.take_written(stage)
    }
    fn supports_create_dirs(&self, root: &str) -> bool {
        !crate::share::fs_capabilities::is_synthetic_container(root) && self.create_dirs_offered()
    }
    fn create_dirs(&self, paths: &[String]) -> io::Result<Vec<io::Result<bool>>> {
        self.create_dirs_bulk(paths)
    }
}
