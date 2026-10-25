//! An outgoing Exec start of this process: its input, its events and what
//! the host told beside them (D.2.2: the access view of a host of this
//! version, and a pending confirmation until the start begins). The daemon's
//! Exec stream to the CLI carries only events (W10: `exec_ipc.rs` stays as it
//! is), so the notices of the running starts are kept here by Exec id and
//! read through the daemon's `Access` IPC (`access_ipc.rs`).
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use tokio::sync::watch;

use super::core::eio;
use super::exec_access_view::ExecClientNotice;
use super::exec_client::{ExecClientEvent, ExecClientInput, ExecClientSession};
use super::exec_types::ExecId;
use super::node::ShareIrohNode;

type Notices = Mutex<HashMap<ExecId, watch::Receiver<ExecClientNotice>>>;

fn notices() -> &'static Notices {
    static NOTICES: OnceLock<Notices> = OnceLock::new();
    NOTICES.get_or_init(Notices::default)
}

/// The notice of one session in `notices`, removed when the session goes.
struct NoticeEntry {
    exec_id: ExecId,
    notice: watch::Receiver<ExecClientNotice>,
}

impl NoticeEntry {
    fn register(exec_id: &ExecId, notice: &watch::Receiver<ExecClientNotice>) -> Self {
        notices()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(exec_id.clone(), notice.clone());
        Self {
            exec_id: exec_id.clone(),
            notice: notice.clone(),
        }
    }
}

impl Drop for NoticeEntry {
    fn drop(&mut self) {
        let mut notices = notices().lock().unwrap_or_else(PoisonError::into_inner);
        // Only this session's own entry: an id is random, but never trust it.
        if notices
            .get(&self.exec_id)
            .is_some_and(|current| current.same_channel(&self.notice))
        {
            notices.remove(&self.exec_id);
        }
    }
}

#[derive(Clone)]
pub(crate) struct ShareExecInput {
    node: Arc<ShareIrohNode>,
    sender: super::exec_client::ExecInputSender,
}

pub(crate) struct ShareExecSession {
    node: Arc<ShareIrohNode>,
    inner: ExecClientSession,
    _notice: NoticeEntry,
}

impl ShareExecSession {
    pub(super) fn new(node: Arc<ShareIrohNode>, inner: ExecClientSession) -> Self {
        let notice = NoticeEntry::register(&inner.exec_id, &inner.notice);
        Self {
            node,
            inner,
            _notice: notice,
        }
    }

    /// What the host of the outgoing start `exec_id` of this process told
    /// beside its events; `None` once its session ended.
    pub(crate) fn notice_of(exec_id: &ExecId) -> Option<ExecClientNotice> {
        notices()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(exec_id)
            .map(|notice| notice.borrow().clone())
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

    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.node
            .block_on(&mut self.inner.task)
            .map_err(eio)?
            .map(|_| ())
            .map_err(eio)
    }
}

impl Drop for ShareExecSession {
    fn drop(&mut self) {
        // Never abort the worker: it owns the retainer until its authenticated
        // terminal exchange or transport shutdown. Cancellation has its own
        // signal and cannot be trapped behind the bounded stdin queue.
        self.inner.input.cancel();
    }
}

impl ShareExecInput {
    pub(crate) fn send(&self, input: ExecClientInput) -> io::Result<()> {
        self.node.block_on(self.sender.send(input)).map_err(eio)
    }
}
