use super::agent_error::agent_error;
use super::backend::AgentBackend;
use crate::{agent_proto::Frame, vfs::VfsResult};
use std::{io, time::Duration};

impl AgentBackend {
    pub(super) fn walk_tree_impl(
        &self,
        root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        self.walk_tree_with_budget(root, on_progress, None)
    }

    pub(super) fn walk_tree_with_budget(&self, root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
        budget: Option<crate::agent_proto::TreeDecodeBudget>) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        let lease = self.pool.lease();
        let mux = lease.mux()?;
        let (id, rx) = mux.register_with_tree_budget(budget);
        let result = (|| {
            mux.send(id, Frame::WalkTree(root.to_string()))?;
            let mut last = (0u64, 0u64);
            loop {
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(Frame::Progress { done, total }) => {
                        last = (done, total);
                        if !on_progress(done, total) {
                            let _ = mux.send(id, Frame::Cancel);
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "agent tree walk canceled",
                            ));
                        }
                    }
                    Ok(Frame::Tree(node)) => return Ok(Some(node)),
                    Ok(Frame::Err(error)) if error == crate::agent_proto::TREE_BUDGET_ERROR =>
                        return Err(io::Error::new(io::ErrorKind::OutOfMemory, error)),
                    Ok(Frame::Err(error)) => return Err(agent_error(error)),
                    Ok(other) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent tree-walk reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if !on_progress(last.0, last.1) {
                            let _ = mux.send(id, Frame::Cancel);
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "agent tree walk canceled",
                            ));
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent tree walk stream closed",
                        ));
                    }
                }
            }
        })();
        mux.unregister(id);
        result
    }
}
