//! Concurrent transfers. Every transfer runs in its own worker with its own
//! progress and cancellation and starts at once: there is no fixed number of
//! transfers. Transfers on one connection share it through that connection's
//! flow (fair turns, adaptive concurrency), transfers on different
//! connections do not compete at all.
use super::job::TransferJob;
use super::types::{ResolvedRoot, TransferIssue, TransferKind, TransferMsg, TransferProgress};
use super::{copy_remote_paths_progress, download_paths_progress};
use super::{upload_pairs_progress, upload_paths_progress};
use crate::types::FilterDef;
use crossbeam_channel::{unbounded, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One transfer the user asked for, before a worker exists for it.
pub enum TransferRequest {
    Upload {
        paths: Vec<String>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    },
    /// Filtered clipboard payloads keep their relative paths.
    UploadPairs {
        pairs: Vec<(String, String)>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    },
    Download {
        backend: crate::vfs::BackendHandle,
        files: Vec<String>,
        dest_local: String,
        filter: Option<(FilterDef, String)>,
    },
    RemoteCopy {
        src: crate::vfs::BackendHandle,
        files: Vec<String>,
        tgt: crate::vfs::BackendHandle,
        dest_root: String,
        filter: Option<(FilterDef, String)>,
    },
    /// Any source to any target through the streaming engine.
    Job(Box<TransferJob>),
}

impl TransferRequest {
    pub fn kind(&self) -> TransferKind {
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => TransferKind::Upload,
            Self::Download { .. } => TransferKind::Download,
            Self::RemoteCopy { .. } => TransferKind::RemoteCopy,
            Self::Job(job) => job.kind(),
        }
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::Upload { paths, .. } => paths.len(),
            Self::UploadPairs { pairs, .. } => pairs.len(),
            Self::Download { files, .. } | Self::RemoteCopy { files, .. } => files.len(),
            Self::Job(job) => job.items.len(),
        }
    }

    fn same_server(&self) -> bool {
        match self {
            Self::RemoteCopy { src, tgt, .. } => Arc::ptr_eq(src, tgt),
            _ => false,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => "Lade hoch",
            Self::Download { .. } => "Lade herunter",
            Self::RemoteCopy { .. } if self.same_server() => "Kopiere remote",
            Self::RemoteCopy { .. } => "Uebertrage remote",
            Self::Job(job) => job.kind().label(),
        }
    }

    fn thread_name(&self) -> &'static str {
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => "remote-upload",
            Self::Download { .. } => "remote-download-multi",
            Self::RemoteCopy { .. } => "remote-to-remote",
            Self::Job(_) => "transfer-job",
        }
    }

    /// The notice shown when the transfer is admitted.
    pub fn announcement(&self) -> String {
        let n = self.item_count();
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => {
                format!("⬆ Lade {n} Element(e) hoch…")
            }
            Self::Download { .. } => format!("⬇ Lade {n} Element(e) herunter…"),
            Self::RemoteCopy { .. } if self.same_server() => {
                format!("⇄ Übertrage {n} Element(e) (Remote→Remote, serverseitig)…")
            }
            Self::RemoteCopy { .. } => format!("⇄ Übertrage {n} Element(e) (Remote→Remote)…"),
            Self::Job(job) => format!(
                "⇄ {}: {n} Element(e) → {}",
                job.kind().label(),
                job.target_label
            ),
        }
    }

    fn run(self, tx: crossbeam_channel::Sender<TransferMsg>, cancel: Arc<AtomicBool>) {
        match self {
            Self::Upload {
                paths,
                backend,
                dest_root,
            } => upload_paths_progress(&*backend, &paths, &dest_root, &tx, &cancel),
            Self::UploadPairs {
                pairs,
                backend,
                dest_root,
            } => upload_pairs_progress(&*backend, &pairs, &dest_root, &tx, &cancel),
            Self::Download {
                backend,
                files,
                dest_local,
                filter,
            } => download_paths_progress(&*backend, &files, &dest_local, filter, &tx, &cancel),
            Self::RemoteCopy {
                src,
                files,
                tgt,
                dest_root,
                filter,
            } => {
                let same_server = Arc::ptr_eq(&src, &tgt);
                copy_remote_paths_progress(
                    &*src,
                    &files,
                    &*tgt,
                    &dest_root,
                    same_server,
                    filter,
                    &tx,
                    &cancel,
                );
            }
            Self::Job(job) => super::engine::run_job(*job, &tx, &cancel),
        }
    }
}

/// A transfer with a running worker.
pub struct ActiveTransfer {
    pub rx: Receiver<TransferMsg>,
    pub progress: TransferProgress,
    pub cancel: Arc<AtomicBool>,
    /// Joined once the worker reported a terminal message; detached on exit
    /// while a backend call still blocks it.
    pub worker: Option<std::thread::JoinHandle<()>>,
    /// The job an engine transfer runs, kept for "transfer missing files".
    pub job: Option<Box<TransferJob>>,
}

impl ActiveTransfer {
    pub fn canceling(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// A transfer whose worker reached a terminal state.
pub struct FinishedTransfer {
    pub cancel_requested: bool,
    /// `None` when the worker ended without a terminal message.
    pub outcome: Option<(TransferProgress, Vec<String>, bool)>,
    /// The first issues with their paths (the log file has all of them).
    pub issues: Vec<TransferIssue>,
    /// Destination roots the transfer resolved, for a resumed run of `job`.
    pub roots: Vec<ResolvedRoot>,
    pub job: Option<Box<TransferJob>>,
}

pub type LaunchTransfer<'a> = dyn FnMut(TransferRequest) -> Result<ActiveTransfer, String> + 'a;

/// The running transfers.
#[derive(Default)]
pub struct TransferLane {
    pub active: Vec<ActiveTransfer>,
}

impl TransferLane {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_empty()
    }

    /// Starts `request` at once.
    pub fn submit(
        &mut self,
        request: TransferRequest,
        launch: &mut LaunchTransfer<'_>,
    ) -> Result<(), String> {
        self.active.push(launch(request)?);
        Ok(())
    }

    /// Apply progress messages and take every transfer that reached a
    /// terminal state (its worker is joined here).
    pub fn poll(&mut self) -> Vec<FinishedTransfer> {
        let mut finished = Vec::new();
        let mut index = 0;
        while index < self.active.len() {
            let mut terminal: Option<Option<(TransferProgress, Vec<String>, bool)>> = None;
            let mut issues = Vec::new();
            let mut roots = Vec::new();
            for _ in 0..16 {
                match self.active[index].rx.try_recv() {
                    Ok(TransferMsg::Progress(progress)) => {
                        self.active[index].progress = progress;
                    }
                    Ok(TransferMsg::Done {
                        progress,
                        errors,
                        canceled,
                        issues: reported,
                        roots: resolved,
                    }) => {
                        terminal = Some(Some((progress, errors, canceled)));
                        issues = reported;
                        roots = resolved;
                        break;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        terminal = Some(None);
                        break;
                    }
                }
            }
            match terminal {
                Some(outcome) => {
                    let mut transfer = self.active.remove(index);
                    if let Some(worker) = transfer.worker.take() {
                        let _ = worker.join();
                    }
                    finished.push(FinishedTransfer {
                        cancel_requested: transfer.canceling(),
                        outcome,
                        issues,
                        roots,
                        job: transfer.job.take(),
                    });
                }
                None => index += 1,
            }
        }
        finished
    }

    pub fn cancel(&self, index: usize) {
        if let Some(transfer) = self.active.get(index) {
            transfer.request_cancel();
        }
    }

    /// Cancel every running worker.
    pub fn cancel_all(&mut self) {
        for transfer in &self.active {
            transfer.request_cancel();
        }
    }

    pub fn workers_unfinished(&self) -> bool {
        self.active.iter().any(|transfer| {
            transfer
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
        })
    }

    /// Cancel everything and join the workers that already finished; a worker
    /// still blocked in a backend call is detached.
    pub fn shutdown(&mut self) {
        self.cancel_all();
        for mut transfer in self.active.drain(..) {
            if let Some(worker) = transfer.worker.take() {
                if worker.is_finished() {
                    let _ = worker.join();
                }
            }
        }
    }
}

/// Spawn the worker for `request`.
pub fn launch_transfer(request: TransferRequest) -> Result<ActiveTransfer, String> {
    let (tx, rx) = unbounded();
    let job = match &request {
        TransferRequest::Job(job) => Some(job.clone()),
        _ => None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let progress = TransferProgress::new(
        request.kind(),
        request.label(),
        request.item_count() as u64,
        0,
    );
    let worker = std::thread::Builder::new()
        .name(request.thread_name().into())
        .spawn(move || request.run(tx, worker_cancel))
        .map_err(|error| format!("Übertragung konnte nicht gestartet werden: {error}"))?;
    Ok(ActiveTransfer {
        rx,
        progress,
        cancel,
        worker: Some(worker),
        job,
    })
}

#[cfg(test)]
#[path = "lane_tests.rs"]
mod tests;
