//! The streaming transfer engine: runs one job from the first listing to the
//! last published file. Discovery walks the selection in its own thread and
//! feeds a bounded queue; workers start on demand while the connection's flow
//! has a free permit and end when idle, so the first files arrive while
//! folders are still being listed. Every combination of local and remote
//! endpoints runs here; progress goes out every ~150 ms and every issue into
//! the job's log file.
#[path = "batch.rs"]
mod batch;
#[path = "batch_get.rs"]
mod batch_get;
#[path = "discovery.rs"]
mod discovery;
#[path = "download.rs"]
mod download;
#[path = "folders.rs"]
pub(crate) mod folders;
#[path = "issues.rs"]
mod issues;
#[path = "legacy.rs"]
mod legacy;
#[path = "local.rs"]
mod local;
#[path = "ops.rs"]
mod ops;
#[path = "publish.rs"]
mod publish;
#[path = "queue.rs"]
mod queue;
#[path = "remote_copy.rs"]
mod remote_copy;
#[path = "roots.rs"]
mod roots;
#[path = "source.rs"]
mod source;
#[path = "stats.rs"]
mod stats;
#[path = "upload.rs"]
mod upload;
#[path = "util.rs"]
mod util;
#[path = "view.rs"]
mod view;
#[path = "worker.rs"]
mod worker;

use folders::FolderRegister;
pub(crate) use legacy::run_legacy;
pub(crate) use util::{jitter, lock, random_hex, sleep_unless};
pub(crate) use view::{JobView, Side};

use super::access::AccessGate;
use super::engine_names::parent_path;
use super::engine_policy::{BatchSizer, Breaker};
use super::flow::{acquire_pair, flow_for, local_flow, Flow, PermitPair};
use super::job::{JobItems, Layout, TransferJob};
use super::types::{TransferIssue, TransferKind, TransferMsg, TransferProgress};
use crate::types::CopyMode;
use crate::vfs::BatchLimits;
use queue::WorkQueue;
use stats::Stats;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Progress goes out this often (plan: ~150 ms, smooth for the eye without
/// flooding the GUI channel).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
/// An idle worker waits this long for new work before it ends: longer than
/// one listing round trip on common links, so workers survive the gap
/// between two listings of a tree, and short against anything a user sees.
const WORKER_LINGER: Duration = Duration::from_millis(250);
/// Shortest dispatcher wait between two looks at the queue.
const MIN_WAIT: Duration = Duration::from_millis(10);

static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

/// Runs `job` and reports through `tx`; exactly one `TransferMsg::Done`.
pub(crate) fn run_job(
    job: TransferJob,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    run_view(
        JobView::of(&job),
        &|message| {
            let _ = tx.send(message);
        },
        cancel,
    );
}

/// Runs a borrowed job; `report` receives progress and exactly one `Done`.
pub(crate) fn run_view(
    view: JobView<'_>,
    report: &(dyn Fn(TransferMsg) + Sync),
    cancel: &AtomicBool,
) {
    let kind = view.kind();
    let mut first = TransferProgress::new(kind, kind.label(), 0, 0);
    first.source = view.source_label.to_string();
    first.target = view.target_label.to_string();
    first.discovering = true;
    report(TransferMsg::Progress(first.clone()));
    let prepared = view.validate().and_then(|()| prepare(&view, cancel));
    let (folders, mut plan) = match prepared {
        Ok(prepared) => prepared,
        Err(message) => return report_failure(report, first, message, cancel),
    };
    let engine = Engine::new(view, kind, folders, cancel, report, &plan);
    local::move_whole_roots(&engine, &mut plan);
    engine.run(&plan);
    engine.finish(&plan);
}

type Prepared<'a> = (FolderRegister<'a>, roots::RootPlan);

fn prepare<'a>(view: &JobView<'a>, cancel: &AtomicBool) -> Result<Prepared<'a>, String> {
    let target_flow = side_flow(view.target, view.target_dir);
    if view.target.is_local() {
        folders::prepare_local_root(view.target_dir)
            .map_err(|error| format!("Zielordner „{}“: {error}", view.target_dir))?;
    }
    let mut folders = FolderRegister::new(view.target, view.target_dir, target_flow.clone());
    if let Some(planner) = roots::remote_planner(view, &target_flow, cancel)? {
        folders = folders.with_planner(planner);
    }
    let plan = roots::plan(view, &folders)?;
    roots::check_local_targets(view, &plan)?;
    Ok((folders, plan))
}

fn report_failure(
    report: &(dyn Fn(TransferMsg) + Sync),
    mut progress: TransferProgress,
    message: String,
    cancel: &AtomicBool,
) {
    progress.done = true;
    progress.discovering = false;
    progress.errors = 1;
    report(TransferMsg::Done {
        progress,
        errors: vec![message.clone()],
        canceled: cancel.load(Ordering::Acquire),
        issues: vec![TransferIssue {
            path: String::new(),
            message,
        }],
        roots: Vec::new(),
    });
}

fn side_flow(side: Side<'_>, path: &str) -> Arc<Flow> {
    match side {
        Side::Remote(backend) => flow_for(backend, path),
        Side::Local => local_flow(path),
    }
}

fn first_source(items: &JobItems) -> &str {
    match items {
        JobItems::Roots { paths, .. } => paths.first().map(String::as_str),
        JobItems::Pairs(pairs) => pairs.first().map(|pair| pair.source.as_str()),
    }
    .unwrap_or("/")
}

/// One running job.
pub(crate) struct Engine<'a> {
    view: JobView<'a>,
    kind: TransferKind,
    job_id: u64,
    user_cancel: &'a AtomicBool,
    stop: AtomicBool,
    fatal: Mutex<Option<String>>,
    queue: WorkQueue,
    stats: Stats,
    issues: issues::IssueLog,
    folders: FolderRegister<'a>,
    /// Listings of the source side.
    source_flow: Arc<Flow>,
    /// Flows a file operation takes permits of (K2: only the remote flow
    /// between local and remote; both local volumes for local copies).
    op_flows: (Arc<Flow>, Option<Arc<Flow>>),
    gate: Option<Arc<AccessGate>>,
    breaker: Mutex<Breaker>,
    batch: Option<BatchLimits>,
    sizer: Mutex<BatchSizer>,
    /// "Transfer missing files": existing destinations are kept (K8).
    resume: bool,
    /// Source folders of a per-file move, removed at the end when empty.
    moved_dirs: Mutex<Vec<String>>,
    report: &'a (dyn Fn(TransferMsg) + Sync),
}

impl<'a> Engine<'a> {
    fn new(
        view: JobView<'a>,
        kind: TransferKind,
        folders: FolderRegister<'a>,
        cancel: &'a AtomicBool,
        report: &'a (dyn Fn(TransferMsg) + Sync),
        plan: &roots::RootPlan,
    ) -> Self {
        let job_id = NEXT_JOB.fetch_add(1, Ordering::AcqRel);
        let first = first_source(view.items);
        let source_flow = side_flow(view.source, first);
        let target_flow = side_flow(view.target, view.target_dir);
        let op_flows = match (view.source, view.target) {
            (Side::Local, Side::Local) | (Side::Remote(_), Side::Remote(_)) => {
                (source_flow.clone(), Some(target_flow))
            }
            (Side::Local, Side::Remote(_)) => (target_flow, None),
            (Side::Remote(_), Side::Local) => (source_flow.clone(), None),
        };
        let gate = view
            .source
            .is_local()
            .then(|| Arc::new(AccessGate::new(&gate_root(view.items))));
        let batch = match (view.source, view.target) {
            (Side::Local, Side::Remote(backend)) => backend.batch_limits(view.target_dir),
            (Side::Remote(backend), Side::Local) => backend.batch_limits(&parent_path(first)),
            _ => None,
        }
        .filter(|limits| limits.max_files > 1 && limits.max_bytes > 0);
        let engine = Self {
            view,
            kind,
            job_id,
            user_cancel: cancel,
            stop: AtomicBool::new(false),
            fatal: Mutex::new(None),
            queue: WorkQueue::new(),
            stats: Stats::new(),
            issues: issues::IssueLog::new(job_id, issues::log_dir()),
            folders,
            source_flow,
            op_flows,
            gate,
            breaker: Mutex::new(Breaker::default()),
            batch,
            sizer: Mutex::new(BatchSizer::default()),
            resume: view.resume.is_some(),
            moved_dirs: Mutex::new(Vec::new()),
            report,
        };
        for _ in 0..plan.omitted {
            engine.stats.omitted();
        }
        engine
    }

    /// Canceled by the user or stopped by a job-ending problem.
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire) || self.user_cancel.load(Ordering::Acquire)
    }

    /// The flag every wait of this job watches.
    pub(crate) fn stop_flag(&self) -> &AtomicBool {
        &self.stop
    }

    pub(crate) fn halt(&self) {
        self.stop.store(true, Ordering::Release);
        self.queue.clear();
    }

    /// Ends the job with `message` (the first reason wins); what was
    /// transferred stays.
    pub(crate) fn fatal(&self, message: String) {
        let first = {
            let mut fatal = lock(&self.fatal);
            let first = fatal.is_none();
            if first {
                *fatal = Some(message.clone());
            }
            first
        };
        if first {
            self.issues.push("", &message);
        }
        self.halt();
    }

    pub(crate) fn issue(&self, path: &str, message: &str) {
        self.issues.push(path, message);
    }

    /// Permits for one file operation; `None` once the job stops.
    pub(crate) fn acquire(&self) -> Option<PermitPair> {
        acquire_pair(
            &self.op_flows.0,
            self.op_flows.1.as_ref(),
            self.job_id,
            &self.stop,
        )
    }

    fn flows_have_spare(&self) -> bool {
        self.op_flows.0.has_spare() && self.op_flows.1.as_ref().is_none_or(|flow| flow.has_spare())
    }

    /// Records a success for the breaker.
    pub(crate) fn succeeded(&self) {
        lock(&self.breaker).success();
    }

    /// Records a connection failure; true once the run of them shows the
    /// connection is gone.
    pub(crate) fn connection_failed(&self) -> bool {
        let running = self.stats.running();
        lock(&self.breaker).failure(running)
    }

    fn run(&self, plan: &roots::RootPlan) {
        std::thread::scope(|scope| {
            scope.spawn(|| discovery::discover(self, plan));
            let mut last_report: Option<Instant> = None;
            loop {
                if self.user_cancel.load(Ordering::Acquire) && !self.stop.load(Ordering::Acquire) {
                    self.halt();
                }
                let queued = self.queue.view();
                let drained = queued.queued == 0 || self.stopped();
                if !queued.producing && drained && queued.workers == 0 {
                    break;
                }
                let busy = self.stats.running() >= queued.workers;
                let spawn = queued.queued > 0
                    && queued.idle == 0
                    && !self.stopped()
                    && (queued.workers == 0 || (busy && self.flows_have_spare()));
                if spawn {
                    let slot = self.queue.add_worker();
                    scope.spawn(move || worker::run(self, slot));
                    continue;
                }
                let due = last_report.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL);
                if due {
                    (self.report)(TransferMsg::Progress(self.progress(queued.queued)));
                    last_report = Some(Instant::now());
                }
                let waited = last_report.map_or(Duration::ZERO, |at| at.elapsed());
                self.queue
                    .wait_changed(PROGRESS_INTERVAL.saturating_sub(waited).max(MIN_WAIT));
            }
        });
    }

    fn progress(&self, queued: usize) -> TransferProgress {
        let mut progress = self.stats.snapshot(self.kind, self.issues.total());
        progress.source = self.view.source_label.to_string();
        progress.target = self.view.target_label.to_string();
        progress.log_path = self.issues.log_path();
        progress.note = self.note(queued);
        progress
    }

    /// "wartet auf …" while the connection is used up by others and this job
    /// has nothing running; otherwise a fixed limit the connection announces.
    fn note(&self, queued: usize) -> Option<String> {
        if queued > 0 && self.stats.running() == 0 && !self.flows_have_spare() {
            let label = match (self.view.source, self.view.target) {
                (Side::Remote(_), Side::Local) => self.view.source_label,
                _ => self.view.target_label,
            };
            return Some(format!("wartet auf {label}"));
        }
        self.view
            .target
            .backend()
            .and_then(|backend| backend.transfer_hint())
            .or_else(|| {
                self.view
                    .source
                    .backend()
                    .and_then(|backend| backend.transfer_hint())
            })
    }

    fn finish(&self, plan: &roots::RootPlan) {
        local::prune_moved_dirs(self, plan);
        let mut progress = self.progress(0);
        progress.done = true;
        progress.discovering = false;
        progress.active.clear();
        progress.current.clear();
        progress.parallel = 0;
        progress.note = None;
        progress.errors = self.issues.total();
        progress.log_path = self.issues.log_path();
        let (issues, errors) = self.issues.shown();
        (self.report)(TransferMsg::Done {
            progress,
            errors,
            canceled: self.user_cancel.load(Ordering::Acquire),
            issues,
            roots: roots::resolved(plan, &self.folders),
        });
    }

    /// Whether downloaded names get the extension of an exported format.
    pub(crate) fn export_names(&self) -> bool {
        !self.view.source.is_local() && !self.view.source.same_namespace(self.view.target)
    }

    pub(crate) fn is_move(&self) -> bool {
        self.view.mode == CopyMode::Move
    }

    pub(crate) fn keeps_folders(&self) -> bool {
        self.view.filter.is_none() && self.view.layout == Layout::Tree
    }
}

/// The folder a read-access grant for local sources covers.
fn gate_root(items: &JobItems) -> String {
    match items {
        JobItems::Roots {
            base: Some(base), ..
        } => base.clone(),
        _ => parent_path(first_source(items)),
    }
}

#[cfg(test)]
#[path = "test_backend.rs"]
mod test_backend;
#[cfg(test)]
#[path = "test_run.rs"]
mod test_run;
#[cfg(test)]
#[path = "tests_batch.rs"]
mod tests_batch;
#[cfg(test)]
#[path = "tests_flow.rs"]
mod tests_flow;
#[cfg(test)]
#[path = "tests_local.rs"]
mod tests_local;
#[cfg(test)]
#[path = "tests_remote.rs"]
mod tests_remote;
