//! Server-side search and hash walk of the agent (moved out of `backend.rs`
//! unchanged, apart from running on a pooled channel).
use super::agent_error::agent_error;
use super::backend::AgentBackend;
use crate::agent_proto::Frame;
use crate::vfs::VfsResult;
use crossbeam_channel::Sender;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn operation_canceled(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, message)
}

impl AgentBackend {
    pub(super) fn agent_search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: Sender<crate::vfs::SearchHit>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        let lease = self.pool.lease();
        let mux = lease.mux()?;
        let (id, rx) = mux.register();
        let result = (|| {
            mux.send(
                id,
                Frame::Search {
                    root: root.to_string(),
                    spec: spec.clone(),
                },
            )?;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = mux.send(id, Frame::Cancel);
                    return Err(operation_canceled("agent search canceled"));
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(Frame::Match {
                        rel,
                        is_dir,
                        size,
                        mtime_ms,
                    }) => tx
                        .send(crate::vfs::SearchHit {
                            rel,
                            is_dir,
                            size,
                            mtime_ms,
                        })
                        .map_err(|_| {
                            if cancel.load(Ordering::Relaxed) {
                                operation_canceled("agent search canceled")
                            } else {
                                io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    "agent search result receiver closed",
                                )
                            }
                        })?,
                    Ok(Frame::End) => return Ok(true),
                    Ok(Frame::Err(error)) => return Err(agent_error(error)),
                    Ok(other) => {
                        let _ = mux.send(id, Frame::Cancel);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent search reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent search stream closed",
                        ));
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

    pub(super) fn agent_walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: Sender<crate::vfs::HashHit>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        let lease = self.pool.lease();
        let mux = lease.mux()?;
        if !mux.link_aware_hash.load(Ordering::Acquire) {
            return Ok(false);
        }
        let (id, rx) = mux.register();
        let result = (|| {
            mux.send(
                id,
                Frame::WalkHashed {
                    root: root.to_string(),
                    want_hash,
                },
            )?;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = mux.send(id, Frame::Cancel);
                    return Err(operation_canceled("agent hash walk canceled"));
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(Frame::HashEntry {
                        rel,
                        is_dir,
                        size,
                        mtime_ms,
                        md5,
                    }) => tx
                        .send(crate::vfs::HashHit {
                            rel,
                            is_dir,
                            size,
                            mtime_ms,
                            md5,
                        })
                        .map_err(|_| {
                            if cancel.load(Ordering::Relaxed) {
                                operation_canceled("agent hash walk canceled")
                            } else {
                                io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    "agent hash-walk result receiver closed",
                                )
                            }
                        })?,
                    Ok(Frame::End) => return Ok(true),
                    Ok(Frame::Err(error)) => return Err(agent_error(error)),
                    Ok(other) => {
                        let _ = mux.send(id, Frame::Cancel);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent hash-walk reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent hash-walk stream closed",
                        ));
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
}
