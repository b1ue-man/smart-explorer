use std::io;
use std::sync::Arc;

use super::core::eio;
use super::exec_client::{ExecClientEvent, ExecClientInput, ExecClientSession};
use super::exec_types::ExecId;
use super::node::ShareIrohNode;

#[derive(Clone)]
pub(crate) struct ShareExecInput {
    node: Arc<ShareIrohNode>,
    sender: tokio::sync::mpsc::Sender<ExecClientInput>,
}

pub(crate) struct ShareExecSession {
    node: Arc<ShareIrohNode>,
    inner: ExecClientSession,
}

impl ShareExecSession {
    pub(super) fn new(node: Arc<ShareIrohNode>, inner: ExecClientSession) -> Self {
        Self { node, inner }
    }

    pub(crate) fn exec_id(&self) -> &ExecId {
        &self.inner.exec_id
    }

    pub(crate) fn input(&self) -> ShareExecInput {
        ShareExecInput {
            node: self.node.clone(),
            sender: self.inner.input.clone(),
        }
    }

    pub(crate) fn send(&self, input: ExecClientInput) -> io::Result<()> {
        self.input().send(input)
    }

    pub(crate) fn next_event(&mut self) -> io::Result<ExecClientEvent> {
        self.node
            .block_on(self.inner.events.recv())
            .ok_or_else(|| eio("Exec-Client endete ohne Terminalstatus"))
    }

    pub(crate) fn finish(self) -> io::Result<()> {
        self.node
            .block_on(self.inner.task)
            .map_err(eio)?
            .map(|_| ())
            .map_err(eio)
    }
}

impl ShareExecInput {
    pub(crate) fn send(&self, input: ExecClientInput) -> io::Result<()> {
        self.node.block_on(self.sender.send(input)).map_err(eio)
    }
}
