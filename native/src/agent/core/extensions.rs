//! The optional backend extensions over the agent protocol (`ext-v1`). The
//! background service answers every one of them for the Share peer it
//! serves (the peer's host does the work where it can); an SSH agent lists
//! and walks with omissions, finishes stages and flushes next to the data,
//! and everything else of an SSH location goes to its SFTP connection.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::Sender;

use super::agent_error::agent_error;
use super::backend::AgentBackend;
use super::ext_stream::{is_unsupported_error, stream, unexpected, CANCEL_POLL};
use super::ext_wire::{
    apply_progress, durability_to_wire, group_from_wire, is_unsupported, limits_from_wire,
    omission_from_wire, report_from_wire, unsupported,
};
use super::metadata::wire_to_vfs;
use crate::agent_proto::{query, Frame};
use crate::vfs::{
    self as vfs, Backend, BackendExtensions, ChangeNotice, ChangeSignalMode, ChangeSubscription,
    HashWalkItem, HashWalkRequest, RecursiveDeleteFailure, RecursiveDeleteProgress,
    RecursiveDeleteReport, RecycleExpectation, RecycleOutcome, StageFinish, StageFinished,
    TargetLimits, VfsListing, VfsResult, VolumeIdentity,
};

#[path = "content_digest.rs"]
mod content_digest;
#[path = "folder_reservation.rs"]
mod folder_reservation;
#[path = "hash_walk.rs"]
mod hash_walk;
#[path = "extensions_watch.rs"]
mod watch;

/// Deadline of one question (`Query`, `TargetLimits`).
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

impl AgentBackend {
    /// The background service, which answers every extension frame.
    fn serves_peer(&self) -> bool {
        let features = self.features();
        features.service && features.extensions
    }

    /// One question the serving side answers with a number; `0` from a
    /// server without the extension frames.
    pub(super) fn query(&self, kind: u8, path: &str) -> VfsResult<u64> {
        if !self.features().extensions {
            return Ok(0);
        }
        let request = Frame::Query {
            kind,
            path: path.to_string(),
        };
        match self
            .pool
            .lease()
            .safe_call_timeout(request, QUERY_TIMEOUT)?
        {
            Frame::Answer(value) => Ok(value),
            Frame::Err(error) if is_unsupported(&error) => Ok(0),
            Frame::Err(error) => Err(agent_error(error)),
            other => Err(unexpected("query", &other)),
        }
    }

    /// A request that changes something; never repeated on its own.
    fn mutation(&self, request: Frame) -> VfsResult<Frame> {
        let lease = self.pool.lease();
        let (mux, reply) = lease.mutation_call(request)?;
        match reply {
            Frame::Err(error) if is_unsupported(&error) => Err(unsupported()),
            Frame::Err(error) => Err(agent_error(error)),
            Frame::Answer(_) | Frame::StageDone { .. } => Ok(reply),
            other => {
                lease.invalidate(&mux);
                Err(unexpected("extension", &other))
            }
        }
    }
}

impl BackendExtensions for AgentBackend {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        folder_reservation::supported(self, root)
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        folder_reservation::create(self, path)
    }

    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        folder_reservation::identity(self, path)
    }

    fn rename_reserved_directory(&self, source: &str, destination: &str) -> VfsResult<()> {
        if !self.features().folder_reservation {
            return Err(unsupported());
        }
        let lease = self.pool.lease();
        let (mux, reply) = lease.mutation_call(Frame::RenameDirectoryNoReplace {
            src: source.into(),
            dst: destination.into(),
        })?;
        match reply {
            Frame::Ok => Ok(()),
            Frame::Err(error) => {
                Err(Frame::folder_error(&error).unwrap_or_else(|| agent_error(error)))
            }
            frame => {
                lease.invalidate(&mux);
                Err(unexpected("reserved-directory rename", &frame))
            }
        }
    }

    fn file_identity(&self, path: &str) -> VfsResult<Option<String>> {
        folder_reservation::file_identity(self, path)
    }

    fn capacity(&self, root: &str) -> VfsResult<vfs::CapacitySnapshot> {
        // SSH agents keep the existing SFTP provider boundary; only the
        // daemon backend service advertises this authorized metadata frame.
        if !self.features().capacity {
            if self.features().service {
                return Ok(vfs::CapacitySnapshot::unknown(self.state_identity(), root));
            }
            return vfs::capacity(&*self.inner, root);
        }
        match self
            .pool
            .lease()
            .safe_call_timeout(Frame::Capacity(root.into()), QUERY_TIMEOUT)
        {
            Ok(Frame::CapacityData(json)) => serde_json::from_str(&json)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid capacity reply")),
            Ok(Frame::Err(error)) => Err(agent_error(error)),
            Ok(frame) => Err(unexpected("capacity", &frame)),
            Err(error) => Ok(vfs::capacity_failure(
                self.state_identity(),
                root,
                vfs::CapacitySource::Agent,
                &error,
            )),
        }
    }

    fn version_archive_policy(&self) -> vfs::VersionArchivePolicy {
        vfs::version_archive_policy(&*self.inner)
    }

    /// The worker announces a packet of one (Z-P4) only for a peer that
    /// offers it (`backend_server::service_version`).
    fn single_put(&self) -> bool {
        self.version().contains(".single-put-v1")
    }

    /// Packet entries keep their time (`batch-mtime-v1`: the SSH agent, or
    /// the worker for a peer that sets it).
    fn batch_put_times(&self) -> bool {
        let features = self.features();
        features.batch_mtime && features.batches()
    }

    fn previous_state_identities(&self) -> VfsResult<Vec<String>> {
        crate::vfs::previous_state_identities(&*self.inner)
    }

    fn sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String> {
        crate::vfs::sync_child_path(&*self.inner, parent, literal_name)
    }

    fn replace_staged_reversible(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<bool> {
        crate::vfs::replace_staged_reversible(&*self.inner, staged, destination, retained)
    }

    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        if !self.features().extensions {
            return self.list_dir(path).map(VfsListing::complete);
        }
        let mut listing = VfsListing::default();
        let never = AtomicBool::new(false);
        let dots = !self.features().service;
        stream(
            &self.pool,
            Frame::ListTolerant(path.to_string()),
            &never,
            |frame| match frame {
                Frame::DirPart { entries, omitted } => {
                    listing
                        .entries
                        .extend(entries.into_iter().map(|m| wire_to_vfs(m, dots)));
                    listing
                        .omitted
                        .extend(omitted.into_iter().map(omission_from_wire));
                    Ok(())
                }
                other => Err(unexpected("tolerant listing", &other)),
            },
        )?;
        Ok(listing)
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        if !self.features().extensions {
            if self.features().service {
                return Ok(StageFinished::default());
            }
            return vfs::finish_stage(&*self.inner, stage, finish);
        }
        let request = Frame::FinishStage {
            stage: stage.to_string(),
            mtime_ms: finish.mtime_ms,
            mode: finish.mode,
            durability: durability_to_wire(finish.durability),
        };
        match self.mutation(request) {
            Ok(Frame::StageDone {
                mtime_applied,
                durable,
            }) => Ok(StageFinished {
                mtime_applied,
                durable,
            }),
            Ok(other) => Err(unexpected("stage finishing", &other)),
            Err(error) if is_unsupported_error(&error) => Ok(StageFinished::default()),
            Err(error) => Err(error),
        }
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        if !self.features().extensions {
            if self.features().service {
                return Ok(false);
            }
            return vfs::sync_filesystem(&*self.inner, root);
        }
        Ok(self.query(query::SYNC_FILESYSTEM, root)? != 0)
    }

    fn confirm_namespace(&self, parent: &str) -> VfsResult<bool> {
        self.sync_filesystem(parent)
    }

    fn target_limits(&self, root: &str) -> TargetLimits {
        if !self.serves_peer() {
            if self.features().service {
                return TargetLimits::default();
            }
            return vfs::target_limits(&*self.inner, root);
        }
        let request = Frame::TargetLimits(root.to_string());
        match self.pool.lease().safe_call_timeout(request, QUERY_TIMEOUT) {
            Ok(Frame::Limits(limits)) => limits_from_wire(limits),
            _ => TargetLimits::default(),
        }
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        vfs::unix_mode(&*self.inner, path)
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<VolumeIdentity>> {
        vfs::volume_identity(&*self.inner, root)
    }

    fn supports_duplicate_search(&self, root: &str) -> VfsResult<bool> {
        if !self.serves_peer() {
            return Ok(false);
        }
        Ok(self.query(query::DUPLICATE_SEARCH, root)? != 0)
    }

    fn find_duplicates(
        &self,
        root: &str,
        min_bytes: u64,
        progress: &crate::analytics::ReclaimProgress,
    ) -> VfsResult<Option<crate::analytics::DuplicateReport>> {
        if !self.serves_peer() {
            return Ok(None);
        }
        let mut groups = Vec::new();
        let mut summary = None;
        let request = Frame::FindDuplicates {
            root: root.to_string(),
            min_bytes,
        };
        let result = stream(&self.pool, request, &progress.cancel, |frame| match frame {
            Frame::DupProgress(state) => {
                apply_progress(progress, &state);
                Ok(())
            }
            Frame::DupGroup(group) => {
                crate::agent_proto::append_duplicate_part(&mut groups, group);
                Ok(())
            }
            Frame::DupSummary(found) => {
                summary = Some(found);
                Ok(())
            }
            other => Err(unexpected("duplicate search", &other)),
        });
        match result {
            Ok(()) => summary
                .map(|summary| {
                    Some(report_from_wire(
                        groups.into_iter().map(group_from_wire).collect(),
                        summary,
                    ))
                })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "duplicate search ended without its summary",
                    )
                }),
            Err(error) if is_unsupported_error(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn supports_hash_walk(&self, root: &str) -> VfsResult<bool> {
        Ok(self.query(query::HASH_WALK, root)? != 0)
    }

    fn hash_walk(
        &self,
        root: &str,
        request: HashWalkRequest,
        tx: Sender<HashWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        hash_walk::walk(self, root, request, tx, cancel)
    }

    /// Hashed by the agent next to the data; never through the SFTP fallback.
    fn content_digest(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<vfs::ContentDigest>> {
        content_digest::file_digest(self, path, cancel)
    }

    fn supports_recycle(&self, path: &str) -> VfsResult<bool> {
        if !self.serves_peer() {
            if self.features().service {
                return Ok(false);
            }
            return vfs::supports_recycle(&*self.inner, path);
        }
        Ok(self.query(query::RECYCLE, path)? != 0)
    }

    fn recycle(&self, path: &str, expected: &RecycleExpectation) -> VfsResult<RecycleOutcome> {
        if !self.serves_peer() {
            if self.features().service {
                return Err(unsupported());
            }
            return vfs::recycle(&*self.inner, path, expected);
        }
        let request = Frame::Recycle {
            path: path.to_string(),
            size: expected.size,
            sha256: expected.sha256.clone(),
        };
        match self.mutation(request)? {
            Frame::Answer(1) => Ok(RecycleOutcome::Recycled),
            Frame::Answer(2) => Ok(RecycleOutcome::Changed),
            other => Err(unexpected("recycle", &other)),
        }
    }

    fn supports_remove_tree(&self, path: &str) -> VfsResult<bool> {
        self.agent_supports_remove_tree(path)
    }

    fn remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        self.agent_remove_tree(path, cancel, progress)
    }

    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        self.agent_change_signal_mode(root)
    }

    fn change_signal(
        &self,
        root: &str,
        poll_interval: Duration,
        tx: Sender<ChangeNotice>,
    ) -> VfsResult<Option<ChangeSubscription>> {
        self.agent_change_signal(root, poll_interval, tx)
    }
}
