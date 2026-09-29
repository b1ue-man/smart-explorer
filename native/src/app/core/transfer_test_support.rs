//! Test doubles for the transfer list and routing tests: a connection whose
//! namespace identity is chosen by the test (no I/O ever happens through it)
//! and a launcher that hands the test the sending side of each transfer.
use crate::transfer::{
    ActiveTransfer, ResolvedRoot, TransferIssue, TransferMsg, TransferProgress, TransferRequest,
};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use crossbeam_channel::{unbounded, Sender};
use std::io::{Error, ErrorKind, Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// A remote account identified only by `identity`.
pub(in crate::app) struct FakeRemote(pub(in crate::app) &'static str);

pub(in crate::app) fn remote(identity: &'static str) -> BackendHandle {
    Arc::new(FakeRemote(identity))
}

fn unused() -> Error {
    Error::new(ErrorKind::Unsupported, "Testverbindung ohne Inhalt")
}

impl Backend for FakeRemote {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn namespace_identity(&self) -> String {
        self.0.to_string()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Err(unused())
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Err(unused())
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(unused())
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(unused())
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(unused())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(unused())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(unused())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(unused())
    }
}

/// Launches nothing: every request becomes an active transfer whose messages
/// the test sends through the returned senders, in launch order.
#[derive(Default)]
pub(in crate::app) struct FakeLaunch {
    pub(in crate::app) senders: Vec<Sender<TransferMsg>>,
}

impl FakeLaunch {
    pub(in crate::app) fn launch(
        &mut self,
        request: TransferRequest,
    ) -> Result<ActiveTransfer, String> {
        let (tx, rx) = unbounded();
        self.senders.push(tx);
        let progress = TransferProgress::new(request.kind(), "Test", 0, 0);
        let job = match request {
            TransferRequest::Job(job) => Some(job),
            _ => None,
        };
        Ok(ActiveTransfer {
            rx,
            progress,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
            job,
        })
    }
}

/// The terminal message of a transfer that copied `files_done` of
/// `files_total` files.
pub(in crate::app) fn done(
    files_done: u64,
    files_total: u64,
    canceled: bool,
    issues: Vec<TransferIssue>,
    roots: Vec<ResolvedRoot>,
) -> TransferMsg {
    let mut progress = TransferProgress::new(crate::transfer::TransferKind::Upload, "Test", 0, 0);
    progress.files_done = files_done;
    progress.files_total = files_total;
    progress.errors = issues.len() as u64;
    progress.done = true;
    let errors = issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path, issue.message))
        .collect();
    TransferMsg::Done {
        progress,
        errors,
        canceled,
        issues,
        roots,
    }
}
