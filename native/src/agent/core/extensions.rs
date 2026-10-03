//! The optional backend extensions over the agent protocol (`ext-v1`). The
//! background service answers every one of them for the Share peer it
//! serves (the peer's host does the work where it can); an SSH agent lists
//! and walks with omissions, finishes stages and flushes next to the data,
//! and everything else of an SSH location goes to its SFTP connection.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, Sender};

use super::agent_error::agent_error;
use super::backend::AgentBackend;
use super::ext_wire::{
    algorithm_to_wire, apply_progress, change_from_wire, durability_to_wire, group_from_wire,
    is_unsupported, limits_from_wire, omission_from_wire, report_from_wire, unsupported,
};
use super::metadata::wire_to_vfs;
use super::pool::AgentPool;
use crate::agent_proto::{query, Frame};
use crate::vfs::{
    self as vfs, Backend, BackendExtensions, ChangeNotice, ChangeSignalMode, ChangeSubscription,
    HashWalkEntry, HashWalkItem, HashWalkRequest, RecycleExpectation, RecycleOutcome, StageFinish,
    StageFinished, TargetLimits, VfsListing, VfsResult, VolumeIdentity,
};

/// How often a waiting stream looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(200);
/// Deadline of one question (`Query`, `TargetLimits`).
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

fn unexpected(what: &str, frame: &Frame) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected agent {what} reply: {frame:?}"),
    )
}

/// Sends `request` and hands every reply frame to `take` until `End`. An
/// `Err` frame fails (the serving side's "unsupported" as `Unsupported`), a
/// set `cancel` sends `Cancel` and stops with `Interrupted`. A dead transport
/// ends the wait through the connection's heartbeat, so no frame deadline is
/// needed (a host may hash one large file for a long time).
fn stream(
    pool: &Arc<AgentPool>,
    request: Frame,
    cancel: &AtomicBool,
    mut take: impl FnMut(Frame) -> io::Result<()>,
) -> io::Result<()> {
    let lease = pool.lease();
    let mux = lease.mux()?;
    let (id, rx) = mux.register();
    let result = (|| {
        mux.send(id, request)?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "agent operation canceled",
                ));
            }
            match rx.recv_timeout(CANCEL_POLL) {
                Ok(Frame::End) => return Ok(()),
                Ok(Frame::Err(error)) if is_unsupported(&error) => return Err(unsupported()),
                Ok(Frame::Err(error)) => return Err(agent_error(error)),
                // Keepalive of the service while the peer works.
                Ok(Frame::Progress { .. }) => {}
                Ok(frame) => take(frame)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "agent stream closed",
                    ))
                }
            }
        }
    })();
    if result.is_err() {
        let _ = mux.send(id, Frame::Cancel);
    }
    mux.unregister(id);
    result
}

fn is_unsupported_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported && is_unsupported(&error.to_string())
}

fn send_notice(
    tx: &Sender<ChangeNotice>,
    mut notice: ChangeNotice,
    cancel: &AtomicBool,
) -> io::Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "change subscription canceled",
            ));
        }
        match tx.send_timeout(notice, CANCEL_POLL) {
            Ok(()) => return Ok(()),
            Err(crossbeam_channel::SendTimeoutError::Timeout(pending)) => notice = pending,
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "receiver gone"))
            }
        }
    }
}

impl AgentBackend {
    /// The background service, which answers every extension frame.
    fn serves_peer(&self) -> bool {
        let features = self.features();
        features.service && features.extensions
    }

    /// One question the serving side answers with a number; `0` from a
    /// server without the extension frames.
    fn query(&self, kind: u8, path: &str) -> VfsResult<u64> {
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
        stream(
            &self.pool,
            Frame::ListTolerant(path.to_string()),
            &never,
            |frame| match frame {
                Frame::DirPart { entries, omitted } => {
                    listing.entries.extend(entries.into_iter().map(wire_to_vfs));
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
        if !self.features().extensions {
            return Ok(false);
        }
        let mut items = 0u64;
        let frame = Frame::WalkHashed2 {
            root: root.to_string(),
            algorithm: algorithm_to_wire(request.algorithm),
            min_bytes: request.min_bytes,
        };
        let receiver_gone = || {
            if cancel.load(Ordering::Relaxed) {
                io::Error::new(io::ErrorKind::Interrupted, "agent hash walk canceled")
            } else {
                io::Error::new(io::ErrorKind::BrokenPipe, "hash-walk receiver closed")
            }
        };
        let result = stream(&self.pool, frame, cancel, |frame| {
            let item = match frame {
                Frame::HashEntry {
                    rel,
                    is_dir,
                    size,
                    mtime_ms,
                    md5,
                } => HashWalkItem::Entry(HashWalkEntry {
                    rel,
                    is_dir,
                    size,
                    mtime_ms,
                    digest: md5,
                }),
                Frame::HashOmitted(omitted) => HashWalkItem::Omitted(omission_from_wire(omitted)),
                other => return Err(unexpected("hash walk", &other)),
            };
            items += 1;
            // A full downstream queue must not hide a cancel from the host.
            let mut pending = item;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(receiver_gone());
                }
                match tx.send_timeout(pending, CANCEL_POLL) {
                    Ok(()) => return Ok(()),
                    Err(crossbeam_channel::SendTimeoutError::Timeout(item)) => pending = item,
                    Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                        return Err(receiver_gone())
                    }
                }
            }
        });
        match result {
            Ok(()) => Ok(true),
            Err(error) if items == 0 && is_unsupported_error(&error) => Ok(false),
            Err(error) => Err(error),
        }
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

    fn change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        if !self.serves_peer() {
            if self.features().service {
                return Ok(None);
            }
            return vfs::change_signal_mode(&*self.inner, root);
        }
        Ok(match self.query(query::CHANGE_SIGNAL, root)? {
            1 => Some(ChangeSignalMode::Push),
            2 => Some(ChangeSignalMode::Poll),
            _ => None,
        })
    }

    fn change_signal(
        &self,
        root: &str,
        poll_interval: Duration,
        tx: Sender<ChangeNotice>,
    ) -> VfsResult<Option<ChangeSubscription>> {
        if !self.serves_peer() {
            if self.features().service {
                return Ok(None);
            }
            return vfs::change_signal(&*self.inner, root, poll_interval, tx);
        }
        if self.change_signal_mode(root)?.is_none() {
            return Ok(None);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let pool = self.pool.clone();
        let request = Frame::Watch {
            root: root.to_string(),
            poll_ms: u64::try_from(poll_interval.as_millis()).unwrap_or(u64::MAX),
        };
        let watching = stop.clone();
        let thread = std::thread::Builder::new()
            .name("agent-change-signal".into())
            .spawn(move || {
                let notices = tx.clone();
                let result = stream(&pool, request, &watching, |frame| match frame {
                    Frame::Change(change) => {
                        send_notice(&notices, change_from_wire(change), &watching)
                    }
                    other => Err(unexpected("change signal", &other)),
                });
                if !watching.load(Ordering::Relaxed) {
                    let reason = match result {
                        Ok(()) => "Änderungs-Abo beendet".to_string(),
                        Err(error) => error.to_string(),
                    };
                    let _ = send_notice(&tx, ChangeNotice::Ended(reason), &watching);
                }
            })?;
        Ok(Some(ChangeSubscription::new(WatchGuard {
            stop,
            thread: Some(thread),
        })))
    }
}

/// Ends a change subscription when dropped: the stream cancels and its
/// thread ends within one poll period.
struct WatchGuard {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
