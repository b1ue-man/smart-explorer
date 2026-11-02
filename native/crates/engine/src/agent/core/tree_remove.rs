//! Folder deletion of an agent location in one request (`RemoveTree`). The
//! serving side deletes next to the data: the SSH agent on its filesystem,
//! the background service through the backend it serves (a Share peer's
//! host). The frame goes out only after `Query REMOVE_TREE` answered 1, so an
//! older agent or service (answer 0) never sees it. Without that answer an
//! SSH location asks its SFTP connection, which may delete the folder with
//! pipelined protocol requests; a service connection has no such fallback
//! here (its connection underneath reopens this same service).
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::RecvTimeoutError;

use super::agent_error::agent_error;
use super::backend::AgentBackend;
use super::ext_stream::{unexpected, CANCEL_POLL};
use super::ext_wire::{is_unsupported, unsupported};
use crate::agent_proto::{query, Frame};
use crate::vfs::{
    self as vfs, RecursiveDeleteFailure, RecursiveDeletePhase, RecursiveDeleteProgress,
    RecursiveDeleteReport, RecursiveDeleteStatus, VfsResult,
};

/// The counters the serving side reported last.
#[derive(Clone, Copy, Default)]
struct Counters {
    planned: u64,
    removed: u64,
}

impl Counters {
    fn report(self, status: RecursiveDeleteStatus) -> RecursiveDeleteReport {
        RecursiveDeleteReport {
            status,
            planned: self.planned,
            removed: self.removed,
        }
    }

    fn failure(self, error: io::Error) -> RecursiveDeleteFailure {
        RecursiveDeleteFailure {
            error,
            planned: self.planned,
            removed: self.removed,
        }
    }
}

impl AgentBackend {
    /// Whether the serving side deletes folders in one request; else, for an
    /// SSH location, whether its SFTP connection offers its own tree delete.
    pub(super) fn agent_supports_remove_tree(&self, path: &str) -> VfsResult<bool> {
        if self.query(query::REMOVE_TREE, path)? != 0 {
            return Ok(true);
        }
        if !self.features().service {
            return vfs::supports_remove_tree(&*self.inner, path);
        }
        Ok(false)
    }

    /// Delete the folder `path` with everything below it by the same
    /// decision as `agent_supports_remove_tree`.
    pub(super) fn agent_remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        let none = Counters::default();
        if cancel.load(Ordering::Acquire) {
            return Ok(none.report(RecursiveDeleteStatus::Canceled));
        }
        let on_server = self
            .query(query::REMOVE_TREE, path)
            .map_err(|error| none.failure(error))?
            != 0;
        if on_server {
            return self.remove_tree_on_server(path, cancel, progress);
        }
        if !self.features().service {
            return vfs::remove_tree(&*self.inner, path, cancel, progress);
        }
        Err(none.failure(io::Error::new(
            io::ErrorKind::Unsupported,
            "the background service does not delete whole folders in one operation",
        )))
    }

    /// One `RemoveTree` request: its `Progress` frames become the caller's
    /// progress, `End` completes, `Err` fails with the counters reached and
    /// a set `cancel` sends `Cancel` and reports `Canceled` (entries the
    /// serving side removed so far stay removed).
    fn remove_tree_on_server(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        let mut counters = Counters::default();
        let lease = self.pool.lease();
        let mux = lease.mux().map_err(|error| counters.failure(error))?;
        let (id, rx) = mux.register();
        let outcome = (|| {
            mux.send(id, Frame::RemoveTree(path.to_string()))?;
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(RecursiveDeleteStatus::Canceled);
                }
                match rx.recv_timeout(CANCEL_POLL) {
                    Ok(Frame::Progress { done, total }) => {
                        counters = Counters {
                            planned: total,
                            removed: done,
                        };
                        progress(RecursiveDeleteProgress {
                            phase: RecursiveDeletePhase::Applying,
                            planned: total,
                            removed: done,
                            current: path.to_string(),
                        });
                    }
                    Ok(Frame::End) => return Ok(RecursiveDeleteStatus::Complete),
                    Ok(Frame::Err(error)) if is_unsupported(&error) => return Err(unsupported()),
                    Ok(Frame::Err(error)) => return Err(agent_error(error)),
                    Ok(other) => return Err(unexpected("folder deletion", &other)),
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
        if !matches!(outcome, Ok(RecursiveDeleteStatus::Complete)) {
            // Stops the serving side at its next entry; harmless when the
            // request already ended with `Err`.
            let _ = mux.send(id, Frame::Cancel);
        }
        mux.unregister(id);
        match outcome {
            Ok(status) => Ok(counters.report(status)),
            Err(error) => Err(counters.failure(error)),
        }
    }
}
