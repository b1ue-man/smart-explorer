//! Content search next to the data (`content_search_v1`): the SSH agent
//! searches its files, the background service searches through the backend
//! it serves (a Share peer's own search, or reading the files). The events
//! stream back under the request's credit; a cancel ends the request.
use super::super::agent_error::agent_error;
use super::AgentBackend;
use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary, Frame};
use crate::vfs::VfsResult;
use crossbeam_channel::Sender;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent content search canceled")
}

impl AgentBackend {
    /// `Ok(None)` (before any event) when the server did not announce
    /// `content_search_v1`: the caller then reads the files.
    pub(super) fn agent_content_search(
        &self,
        root: &str,
        query: &ContentQuery,
        tx: Sender<ContentEvent>,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<ContentSummary>> {
        if !self.features().content_search {
            return Ok(None);
        }
        let lease = self.pool.lease();
        let mux = lease.mux()?;
        let (id, rx) = mux.register();
        let result = (|| {
            mux.send(
                id,
                Frame::ContentSearch {
                    root: root.to_string(),
                    query: query.clone(),
                },
            )?;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(Frame::ContentEvent(event)) => tx.send(event).map_err(|_| {
                        if cancel.load(Ordering::Relaxed) {
                            canceled()
                        } else {
                            io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "agent content search receiver closed",
                            )
                        }
                    })?,
                    Ok(Frame::ContentDone(summary)) => return Ok(Some(summary)),
                    Ok(Frame::Err(error)) => return Err(agent_error(error)),
                    Ok(other) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent content search reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent content search stream closed",
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

#[cfg(test)]
#[path = "content_search_tests.rs"]
mod tests;
