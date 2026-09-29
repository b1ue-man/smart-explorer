//! Single-request operations of the transfer engine (`stage-v1`): one
//! directory level, server-side copy into a private stage and discarding an
//! unpublished stage. Servers without the label keep the trait defaults.
use super::agent_error::agent_error;
use super::backend::AgentBackend;
use crate::agent_proto::Frame;
use std::io;

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
    /// copy there and the caller streams instead. A lost reply stays an
    /// error: the copy is never replayed.
    pub(super) fn agent_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
    ) -> io::Result<Option<u64>> {
        let lease = self.pool.lease();
        let (mux, reply) = lease.mutation_call(Frame::CopyToStage {
            src: src.to_string(),
            stage: stage.to_string(),
            size,
        })?;
        match reply {
            Frame::Copied(copied) => Ok(copied),
            Frame::Err(error) => Err(agent_error(error)),
            other => {
                lease.invalidate(&mux);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent stage-copy reply: {other:?}"),
                ))
            }
        }
    }
}
