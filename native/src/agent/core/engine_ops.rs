//! Single-request operations of the transfer engine (`stage-v1`): one
//! directory level, server-side copy into a private stage and discarding an
//! unpublished stage. Servers without the label keep the trait defaults.
use super::agent_error::agent_error;
use super::backend::AgentBackend;
use crate::agent_proto::Frame;
use crossbeam_channel::RecvTimeoutError;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How soon a waiting server copy notices the caller's cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(100);

impl AgentBackend {
    pub(super) fn agent_create_dir(&self, path: &str, exclusive: bool) -> io::Result<()> {
        self.agent_unit_op(Frame::CreateDir {
            path: path.to_string(),
            exclusive,
        })
    }

    pub(super) fn agent_discard_stage(&self, stage: &str) -> io::Result<()> {
        self.agent_unit_op(Frame::DiscardStage(stage.to_string()))
    }

    /// Copy on the server into the new stage; `None` = the server cannot
    /// copy there and the caller streams instead. `cancel` stops the copy on
    /// the agent between blocks; the agent then removes the stage it created.
    /// A lost reply stays an error: the copy is never replayed.
    pub(super) fn agent_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &AtomicBool,
    ) -> io::Result<Option<u64>> {
        let lease = self.pool.lease();
        let mux = lease.mutation_mux()?;
        let (id, rx) = mux.register();
        let result = (|| {
            mux.send(
                id,
                Frame::CopyToStage {
                    src: src.to_string(),
                    stage: stage.to_string(),
                    size,
                },
            )?;
            let mut canceled = false;
            loop {
                if !canceled && cancel.load(Ordering::Relaxed) {
                    // The reply still tells whether the copy finished first.
                    mux.send(id, Frame::Cancel)?;
                    canceled = true;
                }
                match rx.recv_timeout(CANCEL_POLL) {
                    Ok(Frame::Copied(copied)) => return Ok(copied),
                    Ok(Frame::Err(_)) if canceled => {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Server-Kopie abgebrochen",
                        ))
                    }
                    Ok(Frame::Err(error)) => return Err(agent_error(error)),
                    Ok(other) => {
                        lease.invalidate(&mux);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent stage-copy reply: {other:?}"),
                        ));
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        // A lost mutation reply is ambiguous: the copy is not
                        // replayed, only this connection generation closes.
                        if !mux.is_retired() || mux.is_closed() {
                            lease.invalidate(&mux);
                        }
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent stream closed",
                        ));
                    }
                }
            }
        })();
        mux.unregister(id);
        result
    }
}
