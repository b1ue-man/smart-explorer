//! Planned actions run concurrently, as many as the flows of both sides allow
//! (`sync_flows`), instead of a fixed `min(parallelism)`; the user's "max.
//! transfers" (`BisyncOptions::max_transfers`) stays an upper bound when set.
//! Workers grow while actions wait and no worker already waits for a permit,
//! so a run always queues for its turn on a shared flow without parking idle
//! threads on it.
//!
//! Related actions (same path in another letter case, or one path inside the
//! other) run one after another (`apply_groups`). Every action keeps its own
//! safety logic (capture, revalidation, backup, conflict copy); this module
//! only schedules them and repeats an action the peer refused with overload
//! (`sync_overload`).
use super::apply_groups::action_groups;
use super::apply_retry::AttemptError;
use super::sync_overload::{Backoff, Progress};
use super::types::{Action, BisyncStats};
use crate::transfer::engine::sleep_unless;
use crate::transfer::{classify_error, OpOutcome, PermitPair};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread::Scope;
use std::time::Duration;

const MAX_REPORTED_ERRORS: usize = 100;
/// The coordinator looks at the flows at least this often: permits freed by
/// other jobs do not wake it.
const COORDINATOR_SLICE: Duration = Duration::from_millis(50);

/// Blocks until an action may start (its folder prepared, its permits held);
/// `None` once canceled.
pub(super) type Admit<'f> = dyn Fn(&Action) -> Option<PermitPair> + Sync + 'f;
/// Runs one action with its full safety logic.
pub(super) type Execute<'f> = dyn Fn(&Action) -> Result<BisyncStats, AttemptError> + Sync + 'f;

#[derive(Default)]
pub(super) struct PoolReport {
    pub(super) stats: BisyncStats,
    /// The first failures as (action, message).
    pub(super) errors: Vec<(String, String)>,
    /// Actions that completed; only these may enter a new baseline.
    pub(super) completed: Vec<Action>,
}

#[derive(Default)]
struct PoolState {
    next: usize,
    running: usize,
    busy: usize,
    /// Workers between taking work and holding its permits.
    waiting: usize,
    finished: bool,
    report: PoolReport,
}

struct Pool<'p> {
    actions: &'p [Action],
    groups: Vec<Vec<usize>>,
    max_transfers: usize,
    cancel: &'p AtomicBool,
    admit: &'p Admit<'p>,
    execute: &'p Execute<'p>,
    progress: Progress,
    state: Mutex<PoolState>,
    changed: Condvar,
}

/// Applies `actions`; errors are counted and collected, never abort the run.
pub(super) fn run_actions<'p>(
    actions: &'p [Action],
    max_transfers: usize,
    cancel: &'p AtomicBool,
    admit: &'p Admit<'p>,
    execute: &'p Execute<'p>,
) -> PoolReport {
    let pool = Pool {
        actions,
        groups: action_groups(actions),
        max_transfers,
        cancel,
        admit,
        execute,
        progress: Progress::default(),
        state: Mutex::new(PoolState::default()),
        changed: Condvar::new(),
    };
    std::thread::scope(|scope| pool.coordinate(scope));
    pool.state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .report
}

/// An action may run again after overload unless it keeps both versions and
/// its failure may have come after a commit: a keep-both action publishes its
/// conflict copy first, and repeating it after that copy was published would
/// publish a second one. A failure before any commit published nothing, so
/// it repeats like every other action; those capture and revalidate both
/// sides against the plan, so a repeat after an unclear commit reports the
/// drift instead of acting twice.
fn repeatable(action: &Action, error: &AttemptError) -> bool {
    error.before_commit() || !matches!(action, Action::KeepBothAtoB(_) | Action::KeepBothBtoA(_))
}

impl Pool<'_> {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn coordinate<'s>(&'s self, scope: &'s Scope<'s, '_>) {
        loop {
            let mut state = self.lock();
            if !state.finished
                && (self.canceled() || (state.next >= self.groups.len() && state.busy == 0))
            {
                state.finished = true;
                self.changed.notify_all();
            }
            if state.finished && state.running == 0 {
                break;
            }
            if !state.finished && self.may_grow(&state) {
                state.running += 1;
                drop(state);
                if !self.spawn(scope) {
                    // Out of threads: look again after a pause, not in a
                    // busy loop.
                    let state = self.lock();
                    drop(self.wait(state));
                }
                continue;
            }
            drop(self.wait(state));
        }
    }

    fn wait<'g>(&self, state: MutexGuard<'g, PoolState>) -> MutexGuard<'g, PoolState> {
        match self.changed.wait_timeout(state, COORDINATOR_SLICE) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    fn may_grow(&self, state: &PoolState) -> bool {
        let remaining = self.groups.len().saturating_sub(state.next);
        let idle = state.running.saturating_sub(state.busy);
        remaining > idle
            && state.waiting == 0
            && (self.max_transfers == 0 || state.running < self.max_transfers)
    }

    /// Starts one worker; false when the system refused the thread. A panic
    /// ends the run through the worker's `Enlisted` guard and is caught here,
    /// so the scope never re-raises it in the caller.
    fn spawn<'s>(&'s self, scope: &'s Scope<'s, '_>) -> bool {
        let spawned = std::thread::Builder::new()
            .name("sync-apply".to_string())
            .spawn_scoped(scope, move || {
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| self.worker()));
            });
        let Err(error) = spawned else {
            return true;
        };
        let mut state = self.lock();
        state.running -= 1;
        // Without any worker nothing would ever run: report and stop; the
        // actions not run never count as completed.
        if state.running == 0 {
            state.report.stats.errors += 1;
            state.report.errors.push((
                "sync-apply".to_string(),
                format!("worker start failed: {error}"),
            ));
            state.finished = true;
        }
        drop(state);
        self.changed.notify_all();
        false
    }

    fn worker(&self) {
        let mut enlisted = Enlisted {
            pool: self,
            busy: false,
            waiting: false,
        };
        loop {
            let group = {
                let mut state = self.lock();
                if state.finished || self.canceled() || state.next >= self.groups.len() {
                    None
                } else {
                    let group = state.next;
                    state.next += 1;
                    state.busy += 1;
                    state.waiting += 1;
                    Some(group)
                }
            };
            let Some(group) = group else {
                break;
            };
            enlisted.busy = true;
            enlisted.waiting = true;
            self.run_group(&self.groups[group], &mut enlisted);
            enlisted.busy = false;
            self.lock().busy -= 1;
            self.changed.notify_all();
        }
    }

    /// Entered counted as waiting (set when the group was taken).
    fn run_group(&self, group: &[usize], enlisted: &mut Enlisted<'_, '_>) {
        for &position in group {
            if self.canceled() {
                break;
            }
            let action = &self.actions[position];
            // `None`: canceled before the action finished starting.
            let Some(result) = self.attempt(action, enlisted) else {
                break;
            };
            if result.is_ok() {
                self.progress.touch();
            }
            self.record(action, result);
        }
        self.stop_waiting(enlisted);
    }

    /// Runs one action under its permits. Overload gives them back as such,
    /// waits the peer's delay without them and runs the action again while
    /// `Backoff` allows it; `None` once canceled.
    fn attempt(
        &self,
        action: &Action,
        enlisted: &mut Enlisted<'_, '_>,
    ) -> Option<Result<BisyncStats, AttemptError>> {
        let mut backoff = Backoff::new(&self.progress);
        loop {
            if !enlisted.waiting {
                self.lock().waiting += 1;
                enlisted.waiting = true;
            }
            let permits = (self.admit)(action);
            self.stop_waiting(enlisted);
            let permits = permits?;
            let result = (self.execute)(action);
            let delay = match &result {
                Ok(stats) => {
                    permits.progress(stats.bytes);
                    permits.finish(OpOutcome::Done);
                    None
                }
                Err(error) => {
                    permits.finish(classify_error(error.error()));
                    if repeatable(action, error) {
                        backoff.pause(error.error())
                    } else {
                        None
                    }
                }
            };
            let Some(delay) = delay else {
                return Some(result);
            };
            if !sleep_unless(self.cancel, delay) {
                return Some(result);
            }
        }
    }

    fn stop_waiting(&self, enlisted: &mut Enlisted<'_, '_>) {
        if enlisted.waiting {
            enlisted.waiting = false;
            self.lock().waiting -= 1;
            self.changed.notify_all();
        }
    }

    fn record(&self, action: &Action, result: Result<BisyncStats, AttemptError>) {
        let mut state = self.lock();
        let report = &mut state.report;
        match result {
            Ok(stats) => {
                report.stats.a_to_b += stats.a_to_b;
                report.stats.b_to_a += stats.b_to_a;
                report.stats.deleted += stats.deleted;
                report.stats.bytes += stats.bytes;
                report.completed.push(action.clone());
            }
            Err(error) => {
                report.stats.errors += 1;
                if report.errors.len() < MAX_REPORTED_ERRORS {
                    report
                        .errors
                        .push((format!("{action:?}"), error.into_io().to_string()));
                }
            }
        }
    }
}

/// A worker's place in the pool, given back when it ends. A panic in an
/// action gives it back as well and ends the run with an error (the action's
/// outcome is unknown, so it never counts as completed), instead of leaving
/// the coordinator waiting for a worker that is gone.
struct Enlisted<'w, 'p> {
    pool: &'w Pool<'p>,
    /// Holding a group.
    busy: bool,
    /// Counted as waiting for permits.
    waiting: bool,
}

impl Drop for Enlisted<'_, '_> {
    fn drop(&mut self) {
        let mut state = self.pool.lock();
        state.running -= 1;
        if self.busy {
            state.busy -= 1;
        }
        if self.waiting {
            state.waiting -= 1;
        }
        if std::thread::panicking() {
            state.report.stats.errors += 1;
            state.report.errors.push((
                "sync-apply".to_string(),
                "a sync worker stopped unexpectedly; its action's outcome is unknown and the run ends"
                    .to_string(),
            ));
            state.finished = true;
        }
        drop(state);
        self.pool.changed.notify_all();
    }
}

#[cfg(test)]
#[path = "apply_pool_tests.rs"]
mod tests;
