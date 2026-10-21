//! Shared adaptive concurrency per connection or local volume. Every transfer,
//! sync run and Explorer hand-off that uses the same connection takes permits
//! from one `Flow`; its controller (`core/flow_control.rs`) decides how many
//! operations may run at once. Learned limits outlive a single transfer so the
//! next paste on the same connection starts where the last one ended.
//!
//! Permits go round-robin between jobs, so a small paste next to a large
//! transfer is not starved, and listings get one extra reserved slot so
//! discovery never waits behind long file transfers.
//!
//! A flow that knows its connection's identity starts at half of the limit it
//! learned last time (`hints.rs`) and records what it learns again. A refused
//! thread (`learn_ceiling`) lowers the bound of the flow for as long as the
//! flow lives, whatever the backend announces.
use super::flow_control::{FlowControl, OpOutcome};
use super::limiter::OpAccounts;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const WAIT_SLICE: Duration = Duration::from_millis(100);
/// Job id of callers that do not identify a job.
pub const ANONYMOUS_JOB: u64 = 0;

/// Learned limits are recorded for the next start after this long.
const RECORD_INTERVAL_MS: u64 = 60_000;

struct FlowState {
    control: FlowControl,
    in_flight: usize,
    meta_in_flight: usize,
    waiting: HashMap<u64, usize>,
    last_job: Option<u64>,
    /// The bound a refused thread taught (the backend's own bound still
    /// applies on top).
    learned_ceiling: Option<usize>,
    /// Milliseconds callers waited for a permit, in all.
    waited_ms: u64,
    /// Time of the operations that finished here, in all.
    accounts: OpAccounts,
    /// The limit last recorded in the hint file and when.
    recorded_limit: usize,
    recorded_ms: u64,
}

impl FlowState {
    fn waiters(&self) -> usize {
        self.waiting.values().sum()
    }

    fn saturated(&self) -> bool {
        self.waiters() > 0 || self.in_flight >= self.control.limit()
    }

    fn free(&self) -> bool {
        self.in_flight < self.control.limit()
    }

    /// Round robin between jobs: the job that received the last permit lets
    /// another waiting job go first.
    fn my_turn(&self, job: u64) -> bool {
        self.last_job != Some(job) || self.waiting.keys().all(|waiting| *waiting == job)
    }

    fn stop_waiting(&mut self, job: u64) {
        if let Some(count) = self.waiting.get_mut(&job) {
            *count -= 1;
            if *count == 0 {
                self.waiting.remove(&job);
            }
        }
    }
}

/// One adaptive limiter.
pub struct Flow {
    key: String,
    /// The connection's identity across restarts (`Backend::flow_identity`).
    identity: Option<String>,
    epoch: Instant,
    state: Mutex<FlowState>,
    changed: Condvar,
}

/// Current numbers of a flow for progress displays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowSnapshot {
    pub limit: usize,
    /// The bound the limit may not pass (the backend's, or a lower one a
    /// refused thread taught).
    pub ceiling: usize,
    pub in_flight: usize,
    /// Demand filled the limit during the last stretch observed.
    pub saturated: bool,
    /// The concurrency has stopped growing (see `FlowFacts::plateau`).
    pub plateau: bool,
    /// Milliseconds callers waited for a permit of this flow, in all.
    pub waited_ms: u64,
    /// The time accounts of the operations that finished here, in all.
    pub accounts: OpAccounts,
}

impl Flow {
    /// A flow; `start` is the limit it begins at (half of what its
    /// connection learned last time), if one is known.
    pub(super) fn new(
        key: String,
        identity: Option<String>,
        start: Option<usize>,
        ceiling: usize,
    ) -> Self {
        let control = match start {
            Some(start) => FlowControl::with_start(ceiling, 0, start),
            None => FlowControl::new(ceiling, 0),
        };
        let recorded_limit = control.limit();
        Self {
            key,
            identity,
            epoch: Instant::now(),
            state: Mutex::new(FlowState {
                control,
                in_flight: 0,
                meta_in_flight: 0,
                waiting: HashMap::new(),
                last_job: None,
                learned_ceiling: None,
                waited_ms: 0,
                accounts: OpAccounts::default(),
                recorded_limit,
                recorded_ms: 0,
            }),
            changed: Condvar::new(),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn lock(&self) -> MutexGuard<'_, FlowState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn tick(&self, state: &mut FlowState) {
        let saturated = state.saturated();
        state.control.tick(self.now_ms(), saturated);
    }

    fn wait<'a>(&self, state: MutexGuard<'a, FlowState>) -> MutexGuard<'a, FlowState> {
        let mut state = match self.changed.wait_timeout(state, WAIT_SLICE) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
        self.tick(&mut state);
        state
    }

    /// Waits for a permit without naming a job; `None` once `cancel` is set.
    pub fn acquire(self: &Arc<Self>, cancel: &AtomicBool) -> Option<FlowPermit> {
        self.acquire_for(ANONYMOUS_JOB, cancel)
    }

    /// Waits for a permit for `job`, taking turns with other jobs.
    pub fn acquire_for(self: &Arc<Self>, job: u64, cancel: &AtomicBool) -> Option<FlowPermit> {
        let mut state = self.lock();
        self.tick(&mut state);
        *state.waiting.entry(job).or_insert(0) += 1;
        let began = Instant::now();
        loop {
            if cancel.load(Ordering::Acquire) {
                state.stop_waiting(job);
                drop(state);
                self.changed.notify_all();
                return None;
            }
            if state.free() && state.my_turn(job) {
                break;
            }
            state = self.wait(state);
        }
        state.waited_ms = state
            .waited_ms
            .saturating_add(began.elapsed().as_millis() as u64);
        state.stop_waiting(job);
        state.in_flight += 1;
        state.last_job = Some(job);
        Some(FlowPermit::new(self.clone(), false))
    }

    /// A permit for a listing: an ordinary one when free, otherwise the one
    /// slot reserved for metadata, so discovery never waits behind transfers.
    pub fn acquire_meta(self: &Arc<Self>, cancel: &AtomicBool) -> Option<FlowPermit> {
        let mut state = self.lock();
        self.tick(&mut state);
        loop {
            if cancel.load(Ordering::Acquire) {
                return None;
            }
            if state.free() {
                state.in_flight += 1;
                return Some(FlowPermit::new(self.clone(), false));
            }
            if state.meta_in_flight == 0 {
                state.meta_in_flight += 1;
                return Some(FlowPermit::new(self.clone(), true));
            }
            state = self.wait(state);
        }
    }

    /// A permit only if one is free right now.
    pub fn try_acquire(self: &Arc<Self>) -> Option<FlowPermit> {
        let mut state = self.lock();
        self.tick(&mut state);
        if !state.free() {
            return None;
        }
        state.in_flight += 1;
        Some(FlowPermit::new(self.clone(), false))
    }

    /// Waits until a permit is free (without taking it); false on cancel.
    fn wait_for_spare(&self, cancel: &AtomicBool) -> bool {
        let mut state = self.lock();
        loop {
            if cancel.load(Ordering::Acquire) {
                return false;
            }
            if state.free() {
                return true;
            }
            state = self.wait(state);
        }
    }

    /// Whether another operation could start now without waiting.
    pub fn has_spare(&self) -> bool {
        self.lock().free()
    }

    pub fn snapshot(&self) -> FlowSnapshot {
        let mut state = self.lock();
        self.tick(&mut state);
        let facts = state.control.take_facts();
        FlowSnapshot {
            limit: state.control.limit(),
            ceiling: state.control.ceiling(),
            in_flight: state.in_flight + state.meta_in_flight,
            saturated: facts.saturated,
            plateau: facts.plateau,
            waited_ms: state.waited_ms,
            accounts: state.accounts,
        }
    }

    /// The bound the backend announces; a bound a refused thread taught
    /// stays below it.
    pub(super) fn set_ceiling(&self, ceiling: usize) {
        let mut state = self.lock();
        let ceiling = state
            .learned_ceiling
            .map_or(ceiling, |bound| bound.min(ceiling));
        if state.control.ceiling() != ceiling {
            state.control.set_ceiling(ceiling);
            drop(state);
            self.changed.notify_all();
        }
    }

    /// The process refused to start a thread while `running` operations ran
    /// on this flow: overload. The limit halves and the bound drops to what
    /// ran (at least one), for as long as the flow lives.
    pub(crate) fn learn_ceiling(&self, running: usize) {
        let now = self.now_ms();
        let mut state = self.lock();
        state.control.thread_refused(running, now);
        let bound = state.control.ceiling();
        state.learned_ceiling = Some(
            state
                .learned_ceiling
                .map_or(bound, |known| known.min(bound)),
        );
        drop(state);
        self.changed.notify_all();
    }

    /// Writes the limit this flow settled at to the hint file when it
    /// changed since the last time.
    pub(crate) fn record_hint(&self) {
        let Some(identity) = self.identity.as_deref() else {
            return;
        };
        let mut state = self.lock();
        let settled = state.control.settled_limit();
        if settled == state.recorded_limit {
            return;
        }
        state.recorded_limit = settled;
        state.recorded_ms = self.now_ms();
        drop(state);
        super::hints::record(identity, settled);
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        self.record_hint();
    }
}

/// One running operation. Report streamed bytes with `progress`; `finish`
/// ends it with an outcome, `abandon` returns it unused. Dropping without
/// either counts as failed.
pub struct FlowPermit {
    flow: Arc<Flow>,
    started: Instant,
    finished: bool,
    meta: bool,
}

impl FlowPermit {
    fn new(flow: Arc<Flow>, meta: bool) -> Self {
        Self {
            flow,
            started: Instant::now(),
            finished: false,
            meta,
        }
    }

    pub fn progress(&self, bytes: u64) {
        if bytes > 0 {
            let mut state = self.flow.lock();
            self.flow.tick(&mut state);
            state.control.progress(bytes);
        }
    }

    pub fn finish(mut self, outcome: OpOutcome) {
        self.release(Some(outcome), OpAccounts::default());
    }

    /// Like `finish`, with the time the operation spent by account.
    pub fn finish_with(mut self, outcome: OpOutcome, accounts: OpAccounts) {
        self.release(Some(outcome), accounts);
    }

    /// Returns the permit without feeding the controller (nothing was done).
    pub fn abandon(mut self) {
        self.release(None, OpAccounts::default());
    }

    fn release(&mut self, outcome: Option<OpOutcome>, accounts: OpAccounts) {
        if self.finished {
            return;
        }
        self.finished = true;
        let mut state = self.flow.lock();
        self.flow.tick(&mut state);
        if self.meta {
            state.meta_in_flight = state.meta_in_flight.saturating_sub(1);
        } else {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
        let mut record = None;
        if let Some(outcome) = outcome {
            let now = self.flow.now_ms();
            let latency = self.started.elapsed().as_millis() as u64;
            state.control.finish(outcome, latency, now);
            state.accounts = state.accounts.plus(accounts);
            let settled = state.control.settled_limit();
            if settled != state.recorded_limit
                && now.saturating_sub(state.recorded_ms) >= RECORD_INTERVAL_MS
            {
                state.recorded_limit = settled;
                state.recorded_ms = now;
                record = Some(settled);
            }
        }
        drop(state);
        self.flow.changed.notify_all();
        if let (Some(limit), Some(identity)) = (record, self.flow.identity.as_deref()) {
            super::hints::record(identity, limit);
        }
    }
}

impl Drop for FlowPermit {
    fn drop(&mut self) {
        self.release(Some(OpOutcome::Failed), OpAccounts::default());
    }
}

/// Permits on one or two flows (source and target of a remote-to-remote
/// copy). The second is only taken when free; otherwise the first goes back
/// and the caller waits for the second, so no permit sits idle while another
/// connection is the bottleneck and opposite transfers cannot deadlock.
pub struct PermitPair {
    first: FlowPermit,
    second: Option<FlowPermit>,
}

impl PermitPair {
    pub fn progress(&self, bytes: u64) {
        self.first.progress(bytes);
        if let Some(second) = &self.second {
            second.progress(bytes);
        }
    }

    pub fn finish(self, outcome: OpOutcome) {
        self.finish_with(outcome, OpAccounts::default());
    }

    /// Like `finish`, with the time the operation spent by account; both
    /// flows of a copy between two connections book it.
    pub fn finish_with(self, outcome: OpOutcome, accounts: OpAccounts) {
        let Self { first, second } = self;
        first.finish_with(outcome, accounts);
        if let Some(second) = second {
            second.finish_with(outcome, accounts);
        }
    }

    /// Returns the permits without feeding the controllers: the operation
    /// found nothing to do (a part another worker took first).
    pub fn abandon(self) {
        let Self { first, second } = self;
        first.abandon();
        if let Some(second) = second {
            second.abandon();
        }
    }
}

pub fn acquire_pair(
    one: &Arc<Flow>,
    other: Option<&Arc<Flow>>,
    job: u64,
    cancel: &AtomicBool,
) -> Option<PermitPair> {
    let Some(other) = other.filter(|other| other.key != one.key) else {
        return one.acquire_for(job, cancel).map(|first| PermitPair {
            first,
            second: None,
        });
    };
    let (low, high) = if one.key <= other.key {
        (one, other)
    } else {
        (other, one)
    };
    loop {
        let first = low.acquire_for(job, cancel)?;
        if let Some(second) = high.try_acquire() {
            return Some(PermitPair {
                first,
                second: Some(second),
            });
        }
        first.abandon();
        if !high.wait_for_spare(cancel) {
            return None;
        }
    }
}

pub use super::flow_registry::{classify_error, flow, flow_for, flow_with_identity, local_flow};

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
