//! Planned actions run concurrently, as many as the flows of both sides allow
//! (`sync_flows`), instead of a fixed `min(parallelism)`; the user's "max.
//! transfers" (`BisyncOptions::max_transfers`) stays an upper bound when set.
//! Workers grow while actions wait and no worker already waits for a permit,
//! so a run always queues for its turn on a shared flow without parking idle
//! threads on it.
//!
//! Actions whose paths differ only in letter case run one after another in
//! plan order: a case-insensitive side sees them as one file (a rename that
//! only changes case deletes one spelling and copies the other). Every action
//! keeps its own safety logic (capture, revalidation, backup, conflict copy);
//! this module only schedules them.
use super::apply_retry::AttemptError;
use super::types::{Action, BisyncStats};
use crate::transfer::{classify_error, OpOutcome, PermitPair};
use std::collections::HashMap;
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
        groups: case_groups(actions),
        max_transfers,
        cancel,
        admit,
        execute,
        state: Mutex::new(PoolState::default()),
        changed: Condvar::new(),
    };
    std::thread::scope(|scope| pool.coordinate(scope));
    pool.state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .report
}

/// Action indices grouped by their case-folded path, in plan order.
fn case_groups(actions: &[Action]) -> Vec<Vec<usize>> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (position, action) in actions.iter().enumerate() {
        let key = rel_of(action).to_lowercase();
        match index.get(&key) {
            Some(&group) => groups[group].push(position),
            None => {
                index.insert(key, groups.len());
                groups.push(vec![position]);
            }
        }
    }
    groups
}

fn rel_of(action: &Action) -> &str {
    match action {
        Action::CopyAtoB(rel)
        | Action::CopyBtoA(rel)
        | Action::FinalizeMoveAtoB(rel)
        | Action::FinalizeMoveBtoA(rel)
        | Action::DeleteA(rel)
        | Action::DeleteB(rel)
        | Action::KeepBothAtoB(rel)
        | Action::KeepBothBtoA(rel) => rel,
    }
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

    /// Starts one worker; false when the system refused the thread.
    fn spawn<'s>(&'s self, scope: &'s Scope<'s, '_>) -> bool {
        let spawned = std::thread::Builder::new()
            .name("sync-apply".to_string())
            .spawn_scoped(scope, move || self.worker());
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
            self.run_group(&self.groups[group]);
            self.lock().busy -= 1;
            self.changed.notify_all();
        }
        self.lock().running -= 1;
        self.changed.notify_all();
    }

    /// Entered counted as waiting (set when the group was taken).
    fn run_group(&self, group: &[usize]) {
        let mut waiting = true;
        for &position in group {
            if self.canceled() {
                break;
            }
            let action = &self.actions[position];
            if !waiting {
                self.lock().waiting += 1;
                waiting = true;
            }
            let permits = (self.admit)(action);
            self.lock().waiting -= 1;
            waiting = false;
            self.changed.notify_all();
            // `None`: canceled before the action started; nothing to report.
            let Some(permits) = permits else {
                break;
            };
            let result = (self.execute)(action);
            match &result {
                Ok(stats) => {
                    permits.progress(stats.bytes);
                    permits.finish(OpOutcome::Done);
                }
                Err(error) => permits.finish(classify_error(error.error())),
            }
            self.record(action, result);
        }
        if waiting {
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

#[cfg(test)]
#[path = "apply_pool_tests.rs"]
mod tests;
