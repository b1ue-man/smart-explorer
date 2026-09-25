//! Concurrent remote transfers. Every upload, download or remote-to-remote
//! copy runs in its own worker with its own progress and cancellation. Up to
//! `MAX_ACTIVE_TRANSFERS` run at once; further requests queue in order and
//! start as soon as a slot frees. Transfers to one or several peers (Direct,
//! Room, SFTP, …) therefore share the transport bandwidth through concurrent
//! streams instead of waiting behind a single slot.
use super::types::{TransferKind, TransferMsg, TransferProgress};
use super::{copy_remote_paths_progress, download_paths_progress};
use super::{upload_pairs_progress, upload_paths_progress};
use crate::types::FilterDef;
use crossbeam_channel::{unbounded, Receiver};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Workers running at the same time; more requests queue behind them.
pub const MAX_ACTIVE_TRANSFERS: usize = 6;

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
}

impl TransferRequest {
    pub fn kind(&self) -> TransferKind {
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => TransferKind::Upload,
            Self::Download { .. } => TransferKind::Download,
            Self::RemoteCopy { .. } => TransferKind::RemoteCopy,
        }
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::Upload { paths, .. } => paths.len(),
            Self::UploadPairs { pairs, .. } => pairs.len(),
            Self::Download { files, .. } | Self::RemoteCopy { files, .. } => files.len(),
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
        }
    }

    fn thread_name(&self) -> &'static str {
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => "remote-upload",
            Self::Download { .. } => "remote-download-multi",
            Self::RemoteCopy { .. } => "remote-to-remote",
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
}

impl ActiveTransfer {
    pub fn canceling(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// How a submitted request was admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Started,
    /// Waiting behind running transfers; 1-based queue position.
    Queued(usize),
}

/// A transfer whose worker reached a terminal state.
pub struct FinishedTransfer {
    pub cancel_requested: bool,
    /// `None` when the worker ended without a terminal message.
    pub outcome: Option<(TransferProgress, Vec<String>, bool)>,
}

pub type LaunchTransfer<'a> = dyn FnMut(TransferRequest) -> Result<ActiveTransfer, String> + 'a;

/// Running transfers plus the ordered queue behind them.
pub struct TransferLane {
    pub active: Vec<ActiveTransfer>,
    queued: VecDeque<TransferRequest>,
    capacity: usize,
}

impl TransferLane {
    pub fn new(capacity: usize) -> Self {
        Self {
            active: Vec::new(),
            queued: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_empty() && self.queued.is_empty()
    }

    pub fn queued_len(&self) -> usize {
        self.queued.len()
    }

    /// Start `request` now when a slot is free, otherwise queue it.
    pub fn submit(
        &mut self,
        request: TransferRequest,
        launch: &mut LaunchTransfer<'_>,
    ) -> Result<Admission, String> {
        if self.active.len() >= self.capacity {
            self.queued.push_back(request);
            return Ok(Admission::Queued(self.queued.len()));
        }
        self.active.push(launch(request)?);
        Ok(Admission::Started)
    }

    /// Start queued requests while slots are free. A launch failure drops
    /// only that request and is reported; later requests stay queued.
    pub fn fill(&mut self, launch: &mut LaunchTransfer<'_>) -> Result<(), String> {
        while self.active.len() < self.capacity {
            let Some(request) = self.queued.pop_front() else {
                break;
            };
            self.active.push(launch(request)?);
        }
        Ok(())
    }

    /// Apply progress messages and take every transfer that reached a
    /// terminal state (its worker is joined here).
    pub fn poll(&mut self) -> Vec<FinishedTransfer> {
        let mut finished = Vec::new();
        let mut index = 0;
        while index < self.active.len() {
            let mut terminal: Option<Option<(TransferProgress, Vec<String>, bool)>> = None;
            for _ in 0..16 {
                match self.active[index].rx.try_recv() {
                    Ok(TransferMsg::Progress(progress)) => {
                        self.active[index].progress = progress;
                    }
                    Ok(TransferMsg::Done {
                        progress,
                        errors,
                        canceled,
                    }) => {
                        terminal = Some(Some((progress, errors, canceled)));
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

    /// Cancel every running worker and drop the queue.
    pub fn cancel_all(&mut self) {
        self.queued.clear();
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
    })
}

#[cfg(test)]
#[path = "lane_tests.rs"]
mod tests;
