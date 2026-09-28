//! Shared adaptive concurrency per connection or local volume. Every transfer,
//! sync run and Explorer hand-off that uses the same connection takes permits
//! from one `Flow`; its controller (`core/flow_control.rs`) decides how many
//! operations may run at once. Learned limits outlive a single transfer so the
//! next paste on the same connection starts where the last one ended.
//!
//! Permits go round-robin between jobs, so a small paste next to a large
//! transfer is not starved, and listings get one extra reserved slot so
//! discovery never waits behind long file transfers.
use super::flow_control::{FlowControl, OpOutcome, RESOURCE_CEILING};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Flows unused for this long are forgotten (their connection is gone or idle).
const FLOW_RETENTION: Duration = Duration::from_secs(15 * 60);
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// Job id of callers that do not identify a job.
pub const ANONYMOUS_JOB: u64 = 0;

struct FlowState {
    control: FlowControl,
    in_flight: usize,
    meta_in_flight: usize,
    waiting: HashMap<u64, usize>,
    last_job: Option<u64>,
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
    epoch: Instant,
    state: Mutex<FlowState>,
    changed: Condvar,
}

/// Current numbers of a flow for progress displays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowSnapshot {
    pub limit: usize,
    pub in_flight: usize,
}

impl Flow {
    fn new(key: String, ceiling: usize) -> Self {
        Self {
            key,
            epoch: Instant::now(),
            state: Mutex::new(FlowState {
                control: FlowControl::new(ceiling, 0),
                in_flight: 0,
                meta_in_flight: 0,
                waiting: HashMap::new(),
                last_job: None,
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
        let state = self.lock();
        FlowSnapshot {
            limit: state.control.limit(),
            in_flight: state.in_flight + state.meta_in_flight,
        }
    }

    fn set_ceiling(&self, ceiling: usize) {
        let mut state = self.lock();
        if state.control.ceiling() != ceiling {
            state.control.set_ceiling(ceiling);
            drop(state);
            self.changed.notify_all();
        }
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
        self.release(Some(outcome));
    }

    /// Returns the permit without feeding the controller (nothing was done).
    pub fn abandon(mut self) {
        self.release(None);
    }

    fn release(&mut self, outcome: Option<OpOutcome>) {
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
        if let Some(outcome) = outcome {
            let now = self.flow.now_ms();
            let latency = self.started.elapsed().as_millis() as u64;
            state.control.finish(outcome, latency, now);
        }
        drop(state);
        self.flow.changed.notify_all();
    }
}

impl Drop for FlowPermit {
    fn drop(&mut self) {
        self.release(Some(OpOutcome::Failed));
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
        let Self { first, second } = self;
        first.finish(outcome);
        if let Some(second) = second {
            second.finish(outcome);
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

struct Registry {
    flows: HashMap<String, (Arc<Flow>, Instant)>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            flows: HashMap::new(),
        })
    })
}

/// The flow for `key`, created on first use. `ceiling` is the backend's hard
/// protocol bound (`None` = only the resource guard applies).
pub fn flow(key: String, ceiling: Option<usize>) -> Arc<Flow> {
    let ceiling = ceiling
        .unwrap_or(RESOURCE_CEILING)
        .clamp(1, RESOURCE_CEILING);
    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    registry.flows.retain(|_, (flow, used)| {
        Arc::strong_count(flow) > 1 || now.duration_since(*used) < FLOW_RETENTION
    });
    if let Some((flow, used)) = registry.flows.get_mut(&key) {
        *used = now;
        let flow = flow.clone();
        drop(registry);
        flow.set_ceiling(ceiling);
        return flow;
    }
    let flow = Arc::new(Flow::new(key.clone(), ceiling));
    registry.flows.insert(key, (flow.clone(), now));
    flow
}

/// The flow of the connection serving `path` on `backend`.
pub fn flow_for(backend: &dyn crate::vfs::Backend, path: &str) -> Arc<Flow> {
    flow(backend.flow_key(path), backend.transfer_ceiling(path))
}

/// The flow of the local volume holding `path`.
pub fn local_flow(path: &str) -> Arc<Flow> {
    flow_for(&crate::vfs::LocalBackend::new("/"), path)
}

/// Congestion (the peer asks to slow down) or an ordinary failure. Backends
/// report rate limits and "too many requests/connections" as
/// `vfs::congestion_error`; timeouts count as congestion too. A full disk or
/// an exhausted storage quota (`StorageFull`, `QuotaExceeded`) is permanent,
/// not congestion. The text check covers older services and agents that only
/// send their message.
pub fn classify_error(error: &std::io::Error) -> OpOutcome {
    use std::io::ErrorKind;
    if crate::vfs::congestion_of(error).is_some()
        || matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
        || error
            .to_string()
            .to_ascii_lowercase()
            .contains("too many concurrent")
    {
        OpOutcome::Overload
    } else {
        OpOutcome::Failed
    }
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
