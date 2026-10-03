//! Host jobs survive transport loss; principals and admitted policies bind results.
use crate::{
    analytics::{
        analysis_transfer::{self, AnalysisMessage, SendOptions},
        Progress, ReclaimProgress, ScanPhase,
    },
    share::{
        analysis_admission,
        analysis_spool::{Spool, Writer},
        framing,
        fs_access::FsAccess,
        fs_response::{FsDuplicateMessage, FsDuplicateProgress},
        io_deadline,
        session::PeerPrincipal,
        wire::{FsDuplicateSearch, FsResponse, FsStorageAnalysis},
    },
};
use iroh::endpoint::SendStream;
use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    io,
    sync::atomic::{AtomicU32, Ordering},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub(in crate::share) const CANCEL_CODE: u32 = 0x5345;
const GRACE: Duration = Duration::from_secs(10 * 60);
const HEARTBEAT: Duration = Duration::from_millis(250);
type Key = (PeerPrincipal, String);
type ResultState = Option<Result<Arc<Spool>, Failure>>;

#[derive(Clone)]
struct Failure {
    kind: io::ErrorKind,
    text: String,
}
impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            text: error.to_string(),
        }
    }
}

#[derive(Clone)]
enum Work {
    Analysis(FsStorageAnalysis),
    Duplicates(FsDuplicateSearch),
}
impl Work {
    fn root(&self) -> &str {
        match self {
            Self::Analysis(r) => &r.path,
            Self::Duplicates(r) => &r.path,
        }
    }
    fn id(&self) -> Option<&str> {
        match self {
            Self::Analysis(r) => r.request_id.as_deref(),
            Self::Duplicates(r) => r.request_id.as_deref(),
        }
    }
    fn parameters(&self) -> io::Result<String> {
        let bytes = match self {
            Self::Analysis(r) => serde_json::to_vec(r),
            Self::Duplicates(r) => serde_json::to_vec(r),
        }
        .map_err(io::Error::other)?;
        Ok(format!(
            "{}:{}",
            if matches!(self, Self::Analysis(_)) {
                "analysis"
            } else {
                "duplicates"
            },
            String::from_utf8(bytes).map_err(io::Error::other)?
        ))
    }
}

struct Task {
    work: Work,
    binding: String,
    progress: Progress,
    reclaim: ReclaimProgress,
    _authority: FsAccess,
    _hold: crate::share::analysis_resources::Reservation,
    queue: AtomicU32,
    result: Mutex<ResultState>,
    attached: Mutex<(usize, Instant, u64)>,
}

fn tasks() -> &'static Mutex<HashMap<Key, Arc<Task>>> {
    static TASKS: OnceLock<Mutex<HashMap<Key, Arc<Task>>>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn binding(work: &Work, access: &FsAccess) -> io::Result<String> {
    Ok(format!("{}:{}", work.parameters()?, access.policy_key()?))
}

fn task(work: Work, access: FsAccess, principal: PeerPrincipal) -> io::Result<Arc<Task>> {
    // Freeze exactly the policy used by the admitted request. Revocation
    // cancels this principal through the dispatcher's live policy boundary.
    let access = access.retained_snapshot()?;
    let binding = binding(&work, &access)?;
    let key = if let Some(id) = work.id() {
        if !(16..=64).contains(&id.len())
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Ungültige Analyse-Auftrags-ID",
            ));
        }
        Some((principal.clone(), id.to_owned()))
    } else {
        None
    };
    let mut table = tasks()
        .lock()
        .map_err(|_| io::Error::other("Analyse-Aufträge gesperrt"))?;
    if let Some(existing) = key.as_ref().and_then(|key| table.get(key)) {
        existing._authority.check_read()?;
        if existing.binding != binding || existing.progress.check_cancel().is_err() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Analyse-Auftrag gehört zu anderen Parametern oder Freigaben",
            ));
        }
        return Ok(existing.clone());
    }
    let hold = crate::share::analysis_resources::Reservation::metadata(
        (binding.len() as u64)
            .saturating_mul(4)
            .saturating_add(3 * principal.text_len() as u64)
            .saturating_add(4096),
    )?;
    let progress = Progress::default();
    access.register_cancel(&progress.cancel)?;
    progress.set_phase(ScanPhase::Queued, work.root());
    let reclaim = ReclaimProgress {
        cancel: progress.cancel.clone(),
        ..Default::default()
    };
    let task = Arc::new(Task {
        work,
        binding,
        progress,
        reclaim,
        _authority: access.clone(),
        _hold: hold,
        queue: AtomicU32::new(0),
        result: Mutex::new(None),
        attached: Mutex::new((0, Instant::now(), 0)),
    });
    if let Some(key) = key {
        table.insert(key, task.clone());
    }
    drop(table);
    start(task.clone(), access, principal);
    Ok(task)
}

fn start(task: Arc<Task>, access: FsAccess, principal: PeerPrincipal) {
    tokio::spawn(async move {
        let ticket = analysis_admission::host().enqueue(principal);
        let acquire = ticket.acquire();
        tokio::pin!(acquire);
        let mut tick = tokio::time::interval(HEARTBEAT);
        let permit = loop {
            tokio::select! {
                permit = &mut acquire => break permit,
                _ = tick.tick() => {
                    task.queue.store(ticket.position(), Ordering::Relaxed);
                    if let Err(error) = task.check_authority() { task.finish(Err(error)); return; }
                    if let Err(error) = task.progress.check_cancel() { task.finish(Err(error)); return; }
                }
            }
        };
        task.queue.store(0, Ordering::Relaxed);
        task.progress
            .set_phase(ScanPhase::Preparing, task.work.root());
        let worker = task.clone();
        let spawned = std::thread::Builder::new()
            .name("share-analysis-worker".into())
            .stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES)
            .spawn(move || {
                let _permit = permit;
                let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    build(&worker, &access)
                }))
                .unwrap_or_else(|_| {
                    Err(io::Error::other("Host-Analyse-Worker unerwartet beendet"))
                });
                worker.finish(result);
            });
        if let Err(error) = spawned {
            task.finish(Err(error));
        }
    });
}

fn build(task: &Task, access: &FsAccess) -> io::Result<Spool> {
    task.check_authority()?;
    task.progress.check_cancel()?;
    let writer = RefCell::new(Writer::new()?);
    match &task.work {
        Work::Analysis(request) => {
            let started = Instant::now();
            let mut outcome = crate::share::storage_analysis_host::scan_bounded(
                &request.path,
                access,
                &task.progress,
                request.node_budget,
            );
            let host_scan_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
            task.progress
                .set_phase(ScanPhase::Assembling, &request.path);
            analysis_transfer::send_outcome_with(
                &mut outcome,
                &task.progress,
                SendOptions {
                    host_scan_ms,
                    deflate: request.compress,
                },
                |message| {
                    writer
                        .borrow_mut()
                        .control(FsResponse::Analysis { message })
                },
                |bytes| writer.borrow_mut().frame(framing::TAG_DATA, &bytes),
            )?;
        }
        Work::Duplicates(request) => {
            let report =
                crate::share::storage_duplicate_host::find(request, access, &task.reclaim)?;
            crate::share::storage_duplicate_host::send(report, &task.progress, |message| {
                writer
                    .borrow_mut()
                    .control(FsResponse::Duplicates { message })
            })?;
        }
    }
    writer.into_inner().finish()
}

impl Task {
    fn check_authority(&self) -> io::Result<()> {
        if let Err(error) = self._authority.check_read() {
            self.progress.cancel.store(true, Ordering::Relaxed);
            return Err(error);
        }
        Ok(())
    }
    fn finish(&self, result: io::Result<Spool>) {
        *self.result.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(result.map(Arc::new).map_err(Into::into));
    }
    fn heartbeat(&self) -> FsResponse {
        let queue = self.queue.load(Ordering::Relaxed);
        match &self.work {
            Work::Analysis(_) => {
                let mut state = self.progress.snapshot();
                state.queue_position = queue;
                FsResponse::Analysis {
                    message: AnalysisMessage::Progress { state },
                }
            }
            Work::Duplicates(_) => FsResponse::Duplicates {
                message: FsDuplicateMessage::Progress {
                    state: FsDuplicateProgress::of(
                        &self.reclaim,
                        queue,
                        crate::share::storage_duplicate_host::current(
                            &self.reclaim,
                            self.work.root(),
                        ),
                    ),
                },
            },
        }
    }
}

struct Attachment(Arc<Task>);
impl Attachment {
    fn new(task: Arc<Task>) -> Self {
        let mut attached = task.attached.lock().unwrap_or_else(|p| p.into_inner());
        attached.0 += 1;
        attached.2 = attached.2.wrapping_add(1);
        drop(attached);
        Self(task)
    }
}
impl Drop for Attachment {
    fn drop(&mut self) {
        let mut attached = self.0.attached.lock().unwrap_or_else(|p| p.into_inner());
        attached.0 = attached.0.saturating_sub(1);
        attached.1 = Instant::now();
        let generation = attached.2;
        let detached = attached.0 == 0;
        drop(attached);
        if !detached {
            return;
        }
        if self.0.progress.cancel.load(Ordering::Relaxed) {
            tasks()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|_, item| !Arc::ptr_eq(item, &self.0));
            return;
        }
        if self.0.work.id().is_none() {
            self.0.progress.cancel.store(true, Ordering::Relaxed);
            return;
        }
        let task = self.0.clone();
        tokio::spawn(async move {
            tokio::time::sleep(GRACE).await;
            let attached = task.attached.lock().unwrap_or_else(|p| p.into_inner());
            if attached.0 != 0 || attached.2 != generation || attached.1.elapsed() < GRACE {
                return;
            }
            task.progress.cancel.store(true, Ordering::Relaxed);
            drop(attached);
            tasks()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|_, item| !Arc::ptr_eq(item, &task));
        });
    }
}

pub(in crate::share) async fn serve_analysis(
    send: SendStream,
    request: FsStorageAnalysis,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    serve(send, Work::Analysis(request), access, principal).await
}
pub(in crate::share) async fn serve_duplicates(
    send: SendStream,
    request: FsDuplicateSearch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    serve(send, Work::Duplicates(request), access, principal).await
}

async fn serve(
    mut send: SendStream,
    work: Work,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let task = match task(work, access, principal) {
        Ok(task) => task,
        Err(error) => return framing::reply_err(&mut send, error).await,
    };
    let _attachment = Attachment::new(task.clone());
    let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
    let stopped = send.stopped();
    tokio::pin!(stopped);
    let mut tick = tokio::time::interval(HEARTBEAT);
    let spool = loop {
        task.check_authority()?;
        let result = task
            .result
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(result) = result {
            match result {
                Ok(spool) => break spool,
                Err(error) => {
                    return framing::reply_err(&mut send, io::Error::new(error.kind, error.text))
                        .await
                }
            }
        }
        tokio::select! {
            biased;
            code = &mut stopped => {
                if matches!(code, Ok(Some(code)) if code.into_inner() == u64::from(CANCEL_CODE)) {
                    task.progress.cancel.store(true, Ordering::Relaxed);
                }
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Analyse-Strom beendet"));
            }
            _ = tick.tick() => {
                task.check_authority()?;
                if let Err(error) = io_deadline::run("host analysis heartbeat", framing::reply(&mut send, task.heartbeat())).await {
                    check_explicit_stop(&send, &task).await;
                    return Err(error);
                }
            }
        }
    };
    let mut offset = 0;
    loop {
        task.check_authority()?;
        let local = spool.clone();
        let read = tokio::task::spawn_blocking(move || {
            let mut offset = offset;
            local.frame(&mut offset).map(|frame| (offset, frame))
        });
        let (next, frame) = read.await.map_err(io::Error::other)??;
        offset = next;
        let Some((tag, bytes)) = frame else {
            break;
        };
        task.check_authority()?;
        tokio::select! {
            biased;
            code = &mut stopped => {
                if matches!(code, Ok(Some(code)) if code.into_inner() == u64::from(CANCEL_CODE)) {
                    task.progress.cancel.store(true, Ordering::Relaxed);
                }
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Analyse-Strom beendet"));
            }
            result = io_deadline::run("host analysis result", framing::send_tagged(&mut send, tag, &bytes)) => {
                if let Err(error) = result {
                    check_explicit_stop(&send, &task).await;
                    return Err(error);
                }
            }
        }
    }
    send.finish().map_err(io::Error::other)
}

/// A stopped write can win the race with the explicit STOP future. Poll the
/// recorded code once, so explicit cancellation never enters transport grace.
async fn check_explicit_stop(send: &SendStream, task: &Task) {
    let stopped = send.stopped();
    tokio::pin!(stopped);
    let polled = std::future::poll_fn(|cx| std::task::Poll::Ready(stopped.as_mut().poll(cx))).await;
    if matches!(polled, std::task::Poll::Ready(Ok(Some(code))) if code.into_inner() == u64::from(CANCEL_CODE))
    {
        task.progress.cancel.store(true, Ordering::Relaxed);
    }
}

/// Dispatcher calls this when the relationship's authority is narrowed.
pub(in crate::share) fn cancel_principal(principal: &PeerPrincipal) {
    tasks()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|(peer, _), task| {
            if peer == principal {
                task.progress.cancel.store(true, Ordering::Relaxed);
                false
            } else {
                true
            }
        });
}
