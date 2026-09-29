//! The copy pass of a one-way mirror, parallel to its discovery: source
//! folders are listed concurrently and compared with ONE listing of the
//! matching destination folder (instead of a `stat` per file), and workers
//! copy files while the listing goes on. How many listings and copies run at
//! once is decided by the flows of the pair (`bisync::sync_flows`); every file
//! keeps the checks of the serial pass (see `sync_copy::copy_stream`), links
//! stay protected omissions, and every problem is reported so the caller
//! starts the delete pass only after an error-free copy pass.
//!
//! This file coordinates the workers; what a task does to the shared state
//! lives in `sync_tasks`, the listing of one folder in `sync_scan`.
use super::imp::{record_error, SyncMsg, SyncProgress, SyncStats, WalkBudget};
use super::sync_scan::{scan_directory, DirTask, FileTask, Target};
use crate::bisync::sync_flows::{PairFlows, PairSide};
use crate::bisync::SyncOmissions;
use crate::transfer::engine::folders::FolderRegister;
use crate::transfer::Side;
use crate::vfs::Backend;
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread::Scope;
use std::time::{Duration, Instant};

/// Progress goes out this often, like every transfer (smooth for the eye
/// without flooding the GUI channel).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
/// The coordinator looks at queues and flows at least this often: flows
/// shared with other jobs free permits without waking this pass.
const COORDINATOR_SLICE: Duration = Duration::from_millis(50);
/// Idle workers wait this long for new work before they end: longer than one
/// listing round trip on common links, so workers survive the gap between two
/// listings (the transfer engine's reasoning).
const WORKER_LINGER: Duration = Duration::from_millis(250);
/// Blocked workers look at cancellation at least this often, as the flows'
/// own waits do: a canceled run stops within a tenth of a second.
pub(super) const WAIT_SLICE: Duration = Duration::from_millis(100);

/// What the pass hands back to the mirror driver.
pub(super) struct Report {
    pub(super) stats: SyncStats,
    pub(super) errors: Vec<(String, String)>,
    pub(super) omissions: SyncOmissions,
}

/// Worker counts of one kind (listing or copying).
#[derive(Default)]
pub(super) struct Crew {
    pub(super) running: usize,
    /// Holding a task (listing a folder, copying a file).
    pub(super) busy: usize,
    /// Blocked on a permit, a folder or queue space: no new worker helps.
    pub(super) waiting: usize,
}

impl Crew {
    fn idle(&self) -> usize {
        self.running.saturating_sub(self.busy)
    }
}

/// What a progress message says, apart from the elapsed time.
type ProgressKey = (u64, u64, u64, u64, u64, String);

fn progress_key(progress: &SyncProgress) -> ProgressKey {
    let stats = &progress.stats;
    (
        stats.copied,
        stats.skipped,
        stats.deleted,
        stats.bytes,
        stats.errors,
        progress.current.clone(),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Scanner,
    Copier,
}

pub(super) struct State {
    pub(super) dirs: VecDeque<DirTask>,
    pub(super) files: VecDeque<FileTask>,
    pub(super) scanners: Crew,
    pub(super) copiers: Crew,
    /// Discovery ended early (tree budget exceeded): no further listings.
    pub(super) scan_stopped: bool,
    /// Everything is done or canceled: workers end.
    pub(super) finished: bool,
    pub(super) budget: WalkBudget,
    pub(super) report: Report,
    pub(super) current: String,
}

impl State {
    fn crew(&mut self, kind: Kind) -> &mut Crew {
        match kind {
            Kind::Scanner => &mut self.scanners,
            Kind::Copier => &mut self.copiers,
        }
    }

    fn discovery_over(&self) -> bool {
        self.dirs.is_empty() && self.scanners.busy == 0
    }
}

/// Everything the workers of one pass share.
pub(super) struct Pass<'a> {
    pub(super) src: &'a dyn Backend,
    pub(super) src_root: &'a str,
    pub(super) dst: &'a dyn Backend,
    pub(super) dst_root: &'a str,
    pub(super) dry_run: bool,
    pub(super) cancel: &'a AtomicBool,
    pub(super) flows: PairFlows,
    pub(super) folders: FolderRegister<'a>,
    /// Copies at once when both sides are one remote connection
    /// (`PairFlows::shared_connection_cap`); unbounded otherwise.
    copy_cap: usize,
    pub(super) state: Mutex<State>,
    pub(super) changed: Condvar,
    /// Bytes of running copies, shown in progress before they complete.
    pub(super) streaming: AtomicU64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_pass(
    src: &dyn Backend,
    src_root: &str,
    dst: &dyn Backend,
    dst_root: &str,
    dry_run: bool,
    root: Target,
    cancel: &AtomicBool,
    report: Report,
    tx: &Sender<SyncMsg>,
    start: Instant,
) -> Report {
    let flows = PairFlows::new(src, src_root, dst, dst_root);
    let folders = FolderRegister::new(Side::Remote(dst), dst_root, flows.flow(PairSide::B).clone());
    let copy_cap = flows.shared_connection_cap(src, dst).unwrap_or(usize::MAX);
    let pass = Pass {
        src,
        src_root,
        dst,
        dst_root,
        dry_run,
        cancel,
        flows,
        folders,
        copy_cap,
        state: Mutex::new(State {
            dirs: VecDeque::new(),
            files: VecDeque::new(),
            scanners: Crew::default(),
            copiers: Crew::default(),
            scan_stopped: false,
            finished: false,
            budget: WalkBudget::default(),
            report,
            current: String::new(),
        }),
        changed: Condvar::new(),
        streaming: AtomicU64::new(0),
    };
    {
        let mut guard = pass.lock();
        let state = &mut *guard;
        match state.budget.record(src_root, 0) {
            Ok(()) => state.dirs.push_back(DirTask {
                path: src_root.to_string(),
                rel: String::new(),
                depth: 0,
                target: root,
            }),
            Err(error) => record_error(
                &mut state.report.stats,
                &mut state.report.errors,
                src_root,
                error,
            ),
        }
    }
    std::thread::scope(|scope| pass.coordinate(scope, tx, start));
    let state = pass
        .state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.report
}

impl Pass<'_> {
    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn wait<'g>(
        &self,
        state: MutexGuard<'g, State>,
        limit: Duration,
    ) -> MutexGuard<'g, State> {
        match self.changed.wait_timeout(state, limit) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    /// The user canceled the run (failures seen now are its consequence).
    pub(super) fn canceled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Stop looking at further entries: canceled, finished or out of budget.
    pub(super) fn scan_halted(&self) -> bool {
        if self.canceled() {
            return true;
        }
        let state = self.lock();
        state.finished || state.scan_stopped
    }

    fn coordinate<'s>(&'s self, scope: &'s Scope<'s, '_>, tx: &Sender<SyncMsg>, start: Instant) {
        let mut last_progress = Instant::now();
        let mut last_sent: Option<ProgressKey> = None;
        loop {
            let mut state = self.lock();
            if self.canceled() && !state.finished {
                state.finished = true;
                self.changed.notify_all();
            }
            if state.finished {
                if state.scanners.running == 0 && state.copiers.running == 0 {
                    break;
                }
            } else if state.discovery_over() && state.files.is_empty() && state.copiers.busy == 0 {
                state.finished = true;
                self.changed.notify_all();
                continue;
            } else if let Some(kind) = self.next_worker(&state) {
                state.crew(kind).running += 1;
                drop(state);
                if !self.spawn(scope, kind) {
                    // Out of threads: look again after a pause, not in a
                    // busy loop.
                    drop(self.wait(self.lock(), COORDINATOR_SLICE));
                }
                continue;
            }
            if !state.finished && last_progress.elapsed() >= PROGRESS_INTERVAL {
                let progress = self.progress(&state, start);
                drop(state);
                last_progress = Instant::now();
                // Only news is sent (a stalled pass stays silent), and outside
                // the lock: a full GUI channel never stalls the workers.
                let key = progress_key(&progress);
                if last_sent.as_ref() != Some(&key) {
                    last_sent = Some(key);
                    let _ = tx.send(SyncMsg::Progress(progress));
                }
                continue;
            }
            drop(self.wait(state, COORDINATOR_SLICE));
        }
    }

    /// Workers grow on demand: a listing worker while folders wait and the
    /// flows allow another listing (their limit plus the reserved listing
    /// slot), a copy worker while files wait and none already waits for a
    /// permit. So this pass always queues for its turn on a shared flow but
    /// never parks idle threads on a saturated one.
    fn next_worker(&self, state: &State) -> Option<Kind> {
        let listing_cap = self.flows.limit().saturating_add(1);
        if !state.scan_stopped
            && state.dirs.len() > state.scanners.idle()
            && state.scanners.waiting == 0
            && state.scanners.running < listing_cap
        {
            return Some(Kind::Scanner);
        }
        if state.files.len() > state.copiers.idle()
            && state.copiers.waiting == 0
            && state.copiers.running < self.copy_cap
        {
            return Some(Kind::Copier);
        }
        None
    }

    /// Starts one worker; false (and the failure recorded when none of its
    /// kind is left) when the system refused the thread.
    fn spawn<'s>(&'s self, scope: &'s Scope<'s, '_>, kind: Kind) -> bool {
        let name = match kind {
            Kind::Scanner => "sync-scan",
            Kind::Copier => "sync-copy",
        };
        let spawned = std::thread::Builder::new()
            .name(name.to_string())
            .spawn_scoped(scope, move || match kind {
                Kind::Scanner => self.scanner(),
                Kind::Copier => self.copier(),
            });
        let Err(error) = spawned else {
            return true;
        };
        let mut guard = self.lock();
        let state = &mut *guard;
        state.crew(kind).running -= 1;
        // Without any worker of this kind the pass cannot go on.
        if state.crew(kind).running == 0 {
            record_error(
                &mut state.report.stats,
                &mut state.report.errors,
                name,
                format!("worker start failed: {error}"),
            );
            state.finished = true;
        }
        drop(guard);
        self.changed.notify_all();
        false
    }

    fn progress(&self, state: &State, start: Instant) -> SyncProgress {
        let mut stats = state.report.stats.clone();
        stats.bytes = stats
            .bytes
            .saturating_add(self.streaming.load(Ordering::Relaxed));
        SyncProgress {
            current: state.current.clone(),
            stats,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    fn scanner(&self) {
        self.work(
            Kind::Scanner,
            |state| state.dirs.pop_front(),
            scan_directory,
        );
    }

    fn copier(&self) {
        self.work(
            Kind::Copier,
            |state| state.files.pop_front(),
            Self::copy_file,
        );
    }

    /// One worker: takes tasks while there are any, lingers briefly when more
    /// may come, ends when the pass is over.
    fn work<T>(&self, kind: Kind, take: impl Fn(&mut State) -> Option<T>, run: impl Fn(&Self, T)) {
        let mut idle_since: Option<Instant> = None;
        loop {
            let task = {
                let mut state = self.lock();
                loop {
                    let halted = state.finished
                        || self.canceled()
                        || (kind == Kind::Scanner && state.scan_stopped);
                    if halted {
                        break None;
                    }
                    if let Some(task) = take(&mut *state) {
                        let crew = state.crew(kind);
                        crew.busy += 1;
                        if kind == Kind::Copier {
                            // Counted until its permits are granted.
                            crew.waiting += 1;
                        }
                        break Some(task);
                    }
                    let more_may_come = match kind {
                        Kind::Scanner => state.scanners.busy > 0,
                        Kind::Copier => !state.discovery_over(),
                    };
                    let waited = idle_since.get_or_insert_with(Instant::now).elapsed();
                    if !more_may_come || waited >= WORKER_LINGER {
                        break None;
                    }
                    state = self.wait(state, (WORKER_LINGER - waited).min(WAIT_SLICE));
                }
            };
            let Some(task) = task else {
                break;
            };
            idle_since = None;
            run(self, task);
            self.lock().crew(kind).busy -= 1;
            self.changed.notify_all();
        }
        self.lock().crew(kind).running -= 1;
        self.changed.notify_all();
    }
}
