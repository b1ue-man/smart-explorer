//! Ranged upload to an SSH agent (`range-v1`): the client side of
//! `SegStageOpen`, `WriteAt` and `SegStageFinish` as a `SegmentedStage`.
//!
//! The agent keeps a stage in the table of the connection that opened it, and
//! every SSH exec channel is its own agent process with its own table. So the
//! stage pins the channel it was opened on (`lease` and `mux`): all of its
//! ranges run there, as separate requests with their own credit window. An
//! agent without the label gets none of these frames; the file then goes as
//! one stream.
use super::agent_error::agent_error;
use super::backend::AgentBackend;
use super::mux::Mux;
use super::pool::ChannelLease;
use super::stream::open_write_on;
use crate::agent_proto::Frame;
use crate::vfs::{SegmentedStage, VfsResult};
use std::io::{self, Write};
use std::sync::Arc;

/// Marker of the engine's private upload stages: `<file>.se-upload-<16 hex>`.
const STAGE_MARKER: &str = ".se-upload-";

/// The destination an engine stage name `hint` belongs to, if it has the
/// shape of an upload stage.
fn destination_of(hint: &str) -> Option<&str> {
    let position = hint.rfind(STAGE_MARKER)?;
    let suffix = &hint[position + STAGE_MARKER.len()..];
    let hex = suffix.len() == 16
        && suffix
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    (position > 0 && hex).then(|| &hint[..position])
}

pub(super) struct AgentSegmentedStage {
    lease: Arc<ChannelLease>,
    mux: Arc<Mux>,
    /// The name the agent picked.
    stage: String,
    lz4: bool,
}

impl AgentBackend {
    /// A stage for `size` bytes next to the destination of the engine stage
    /// `stage_hint`; `None` unless the agent announced `range-v1`.
    pub(super) fn agent_open_segmented_stage(
        &self,
        stage_hint: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        let features = self.features();
        let Some(destination) = destination_of(stage_hint).filter(|_| features.ranges) else {
            return Ok(None);
        };
        let lease = Arc::new(self.pool.lease());
        let mux = lease.mutation_mux()?;
        let (id, rx) = mux.register();
        let request = Frame::SegStageOpen {
            path: destination.to_string(),
            size,
        };
        if let Err(error) = mux.send(id, request) {
            mux.unregister(id);
            return Err(error);
        }
        let reply = rx.recv();
        mux.unregister(id);
        match reply {
            Ok(Frame::SegStageReady { stage }) => Ok(Some(Box::new(AgentSegmentedStage {
                lease,
                mux,
                stage,
                lz4: features.lz4,
            }))),
            Ok(Frame::Err(error)) => Err(agent_error(error)),
            Ok(other) => {
                lease.invalidate(&mux);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent reply to the stage request: {other:?}"),
                ))
            }
            Err(_) => {
                lease.invalidate(&mux);
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent connection closed before the stage was made",
                ))
            }
        }
    }
}

impl AgentSegmentedStage {
    /// One request on the stage's channel that is answered with `Ok`.
    fn call(&self, request: Frame) -> io::Result<()> {
        let (id, rx) = self.mux.register();
        if let Err(error) = self.mux.send(id, request) {
            self.mux.unregister(id);
            return Err(error);
        }
        let reply = rx.recv();
        self.mux.unregister(id);
        match reply {
            Ok(Frame::Ok) => Ok(()),
            Ok(Frame::Err(error)) => Err(agent_error(error)),
            Ok(other) => {
                self.lease.invalidate(&self.mux);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent reply: {other:?}"),
                ))
            }
            Err(_) => {
                self.lease.invalidate(&self.mux);
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent connection closed",
                ))
            }
        }
    }
}

impl SegmentedStage for AgentSegmentedStage {
    fn stage_path(&self) -> &str {
        &self.stage
    }

    /// `WriteAt` as its own request on the stage's channel; the writer's
    /// `flush` fails unless exactly `len` bytes came.
    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        let request = Frame::WriteAt {
            stage: self.stage.clone(),
            offset,
            len,
        };
        open_write_on(
            self.lease.clone(),
            self.mux.clone(),
            request,
            Some(len),
            self.lz4,
        )
    }

    fn finish(self: Box<Self>) -> VfsResult<()> {
        self.call(Frame::SegStageFinish {
            stage: self.stage.clone(),
        })
    }

    /// The agent removes the stage and its table entry (`DiscardStage`);
    /// if that does not arrive, it removes the stage when the connection ends.
    fn abort(self: Box<Self>) {
        let _ = self.call(Frame::DiscardStage(self.stage.clone()));
    }
}

#[cfg(test)]
#[path = "segmented_stage_tests.rs"]
mod tests;
