//! Concurrent remote transfers. Every upload, download or remote-to-remote
//! copy runs in its own worker with its own progress and cancellation. Up to
//! `MAX_ACTIVE_TRANSFERS` run at once; further requests queue in order and
//! start as soon as a slot frees. Transfers to one or several peers (Direct,
//! Room, SFTP, …) therefore share the transport bandwidth through concurrent
//! streams instead of waiting behind a single slot.
use super::prelude::*;
use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

/// Workers running at the same time; more requests queue behind them.
pub(in crate::app) const MAX_ACTIVE_TRANSFERS: usize = 6;

/// One transfer the user asked for, before a worker exists for it.
pub(in crate::app) enum TransferRequest {
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
    pub(in crate::app) fn kind(&self) -> TransferKind {
        match self {
            Self::Upload { .. } | Self::UploadPairs { .. } => TransferKind::Upload,
            Self::Download { .. } => TransferKind::Download,
            Self::RemoteCopy { .. } => TransferKind::RemoteCopy,
        }
    }

    pub(in crate::app) fn item_count(&self) -> usize {
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
    pub(in crate::app) fn announcement(&self) -> String {
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
pub(in crate::app) struct ActiveTransfer {
    pub(in crate::app) rx: Receiver<TransferMsg>,
    pub(in crate::app) progress: TransferProgress,
    pub(in crate::app) cancel: Arc<AtomicBool>,
    /// Joined once the worker reported a terminal message; detached on exit
    /// while a backend call still blocks it.
    pub(in crate::app) worker: Option<std::thread::JoinHandle<()>>,
}

impl ActiveTransfer {
    pub(in crate::app) fn canceling(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub(in crate::app) fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// How a submitted request was admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Admission {
    Started,
    /// Waiting behind running transfers; 1-based queue position.
    Queued(usize),
}

/// A transfer whose worker reached a terminal state.
pub(in crate::app) struct FinishedTransfer {
    pub(in crate::app) cancel_requested: bool,
    /// `None` when the worker ended without a terminal message.
    pub(in crate::app) outcome: Option<(TransferProgress, Vec<String>, bool)>,
}

pub(in crate::app) type LaunchTransfer<'a> =
    dyn FnMut(TransferRequest) -> Result<ActiveTransfer, String> + 'a;

/// Running transfers plus the ordered queue behind them.
pub(in crate::app) struct TransferLane {
    pub(in crate::app) active: Vec<ActiveTransfer>,
    queued: VecDeque<TransferRequest>,
    capacity: usize,
}

impl TransferLane {
    pub(in crate::app) fn new(capacity: usize) -> Self {
        Self {
            active: Vec::new(),
            queued: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub(in crate::app) fn is_idle(&self) -> bool {
        self.active.is_empty() && self.queued.is_empty()
    }

    pub(in crate::app) fn queued_len(&self) -> usize {
        self.queued.len()
    }

    /// Start `request` now when a slot is free, otherwise queue it.
    pub(in crate::app) fn submit(
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
    pub(in crate::app) fn fill(&mut self, launch: &mut LaunchTransfer<'_>) -> Result<(), String> {
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
    pub(in crate::app) fn poll(&mut self) -> Vec<FinishedTransfer> {
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

    pub(in crate::app) fn cancel(&self, index: usize) {
        if let Some(transfer) = self.active.get(index) {
            transfer.request_cancel();
        }
    }

    /// Cancel every running worker and drop the queue.
    pub(in crate::app) fn cancel_all(&mut self) {
        self.queued.clear();
        for transfer in &self.active {
            transfer.request_cancel();
        }
    }

    pub(in crate::app) fn workers_unfinished(&self) -> bool {
        self.active.iter().any(|transfer| {
            transfer
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
        })
    }

    /// Cancel everything and join the workers that already finished; a worker
    /// still blocked in a backend call is detached.
    pub(in crate::app) fn shutdown(&mut self) {
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
pub(in crate::app) fn launch_transfer(request: TransferRequest) -> Result<ActiveTransfer, String> {
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

impl App {
    /// Admit a transfer: it starts at once when fewer than
    /// `MAX_ACTIVE_TRANSFERS` run, otherwise it waits in order.
    pub(in crate::app) fn submit_transfer(&mut self, request: TransferRequest) {
        let announcement = request.announcement();
        match self.transfers.submit(request, &mut launch_transfer) {
            Ok(Admission::Started) => {
                self.notice = Some((announcement, Instant::now()));
            }
            Ok(Admission::Queued(position)) => {
                self.notice = Some((
                    format!("{announcement} wartet (Position {position})"),
                    Instant::now(),
                ));
            }
            Err(error) => self.error_msg = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn request() -> TransferRequest {
        TransferRequest::Download {
            backend: Arc::new(crate::vfs::LocalBackend::new("/")),
            files: vec!["/never/run".into()],
            dest_local: "/never/run".into(),
            filter: None,
        }
    }

    /// Workers that finish only when released, so admission is observable.
    struct Releases(Vec<mpsc::Sender<bool>>);

    impl Releases {
        fn launcher(
            &mut self,
        ) -> impl FnMut(TransferRequest) -> Result<ActiveTransfer, String> + '_ {
            move |request| {
                let (tx, rx) = unbounded();
                let (release_tx, release_rx) = mpsc::channel::<bool>();
                self.0.push(release_tx);
                let progress =
                    TransferProgress::new(request.kind(), "test", request.item_count() as u64, 0);
                let done_progress = progress.clone();
                let worker = std::thread::spawn(move || {
                    if release_rx.recv().unwrap_or(false) {
                        let _ = tx.send(TransferMsg::Done {
                            progress: done_progress,
                            errors: Vec::new(),
                            canceled: false,
                        });
                    }
                    // `false` ends the worker without a terminal message.
                });
                Ok(ActiveTransfer {
                    rx,
                    progress,
                    cancel: Arc::new(AtomicBool::new(false)),
                    worker: Some(worker),
                })
            }
        }
    }

    fn wait_finished(lane: &mut TransferLane) -> Vec<FinishedTransfer> {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let finished = lane.poll();
            if !finished.is_empty() || Instant::now() > deadline {
                return finished;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn recursive_filter_task_transfer_lane_runs_up_to_capacity_and_queues_the_rest() {
        let mut releases = Releases(Vec::new());
        let mut lane = TransferLane::new(2);
        {
            let mut launch = releases.launcher();
            assert_eq!(lane.submit(request(), &mut launch), Ok(Admission::Started));
            assert_eq!(lane.submit(request(), &mut launch), Ok(Admission::Started));
            assert_eq!(
                lane.submit(request(), &mut launch),
                Ok(Admission::Queued(1))
            );
            assert_eq!(
                lane.submit(request(), &mut launch),
                Ok(Admission::Queued(2))
            );
        }
        assert_eq!(lane.active.len(), 2);
        assert_eq!(lane.queued_len(), 2);
        assert!(lane.poll().is_empty(), "nothing finished yet");
        assert!(lane.workers_unfinished());

        releases.0[0].send(true).unwrap();
        let finished = wait_finished(&mut lane);
        assert_eq!(finished.len(), 1);
        assert!(finished[0].outcome.is_some() && !finished[0].cancel_requested);
        assert_eq!(lane.active.len(), 1);

        lane.fill(&mut releases.launcher()).unwrap();
        assert_eq!(lane.active.len(), 2, "the next queued request started");
        assert_eq!(lane.queued_len(), 1);

        lane.cancel(0);
        assert!(lane.active[0].canceling() && !lane.active[1].canceling());
        for release in &releases.0[1..] {
            let _ = release.send(true);
        }
        let mut done = 0;
        while done < 2 {
            done += wait_finished(&mut lane).len();
        }
        lane.fill(&mut releases.launcher()).unwrap();
        assert_eq!(lane.active.len(), 1);
        assert_eq!(lane.queued_len(), 0);
        releases.0.last().unwrap().send(true).unwrap();
        assert_eq!(wait_finished(&mut lane).len(), 1);
        assert!(lane.is_idle());
    }

    #[test]
    fn recursive_filter_task_transfer_lane_reports_lost_workers_and_shuts_down() {
        let mut releases = Releases(Vec::new());
        let mut lane = TransferLane::new(1);
        {
            let mut launch = releases.launcher();
            assert_eq!(lane.submit(request(), &mut launch), Ok(Admission::Started));
            assert_eq!(
                lane.submit(request(), &mut launch),
                Ok(Admission::Queued(1))
            );
        }
        // Ending the worker without a terminal message is reported as lost.
        releases.0[0].send(false).unwrap();
        let finished = wait_finished(&mut lane);
        assert_eq!(finished.len(), 1);
        assert!(finished[0].outcome.is_none());

        lane.fill(&mut releases.launcher()).unwrap();
        assert_eq!(lane.active.len(), 1);
        lane.shutdown();
        assert!(
            lane.is_idle(),
            "shutdown drops the queue and running entries"
        );
        let _ = releases.0[1].send(true);
    }
}
