//! Shared adaptive concurrency per connection or local volume. Every transfer,
//! sync run and Explorer hand-off that uses the same connection takes permits
//! from one `Flow`; its controller (`core/flow_control.rs`) decides how many
//! operations may run at once. Learned limits outlive a single transfer so the
//! next paste on the same connection starts where the last one ended.
use super::flow_control::{FlowControl, OpOutcome, RESOURCE_CEILING};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Flows unused for this long are forgotten (their connection is gone or idle).
const FLOW_RETENTION: Duration = Duration::from_secs(15 * 60);
const WAIT_SLICE: Duration = Duration::from_millis(100);

struct FlowState {
    control: FlowControl,
    in_flight: usize,
    waiters: usize,
}

impl FlowState {
    fn saturated(&self) -> bool {
        self.waiters > 0 || self.in_flight >= self.control.limit()
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
                waiters: 0,
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

    /// Waits for a permit; `None` once `cancel` is set.
    pub fn acquire(self: &Arc<Self>, cancel: &AtomicBool) -> Option<FlowPermit> {
        let mut state = self.lock();
        self.tick(&mut state);
        state.waiters += 1;
        loop {
            if cancel.load(Ordering::Acquire) {
                state.waiters -= 1;
                drop(state);
                self.changed.notify_all();
                return None;
            }
            if state.in_flight < state.control.limit() {
                break;
            }
            state = match self.changed.wait_timeout(state, WAIT_SLICE) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
            self.tick(&mut state);
        }
        state.waiters -= 1;
        state.in_flight += 1;
        Some(FlowPermit::new(self.clone()))
    }

    /// A permit only if one is free right now.
    pub fn try_acquire(self: &Arc<Self>) -> Option<FlowPermit> {
        let mut state = self.lock();
        self.tick(&mut state);
        if state.in_flight >= state.control.limit() {
            return None;
        }
        state.in_flight += 1;
        Some(FlowPermit::new(self.clone()))
    }

    /// Whether another operation could start now without waiting.
    pub fn has_spare(&self) -> bool {
        let state = self.lock();
        state.in_flight < state.control.limit()
    }

    pub fn snapshot(&self) -> FlowSnapshot {
        let state = self.lock();
        FlowSnapshot {
            limit: state.control.limit(),
            in_flight: state.in_flight,
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
/// ends it with an outcome. Dropping without `finish` counts as failed.
pub struct FlowPermit {
    flow: Arc<Flow>,
    started: Instant,
    finished: bool,
}

impl FlowPermit {
    fn new(flow: Arc<Flow>) -> Self {
        Self {
            flow,
            started: Instant::now(),
            finished: false,
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
        self.release(outcome);
    }

    fn release(&mut self, outcome: OpOutcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        let mut state = self.flow.lock();
        self.flow.tick(&mut state);
        state.in_flight = state.in_flight.saturating_sub(1);
        let now = self.flow.now_ms();
        let latency = self.started.elapsed().as_millis() as u64;
        state.control.finish(outcome, latency, now);
        drop(state);
        self.flow.changed.notify_all();
    }
}

impl Drop for FlowPermit {
    fn drop(&mut self) {
        self.release(OpOutcome::Failed);
    }
}

/// Permits on one or two flows (source and target of a remote-to-remote
/// copy), acquired in key order so opposite transfers cannot deadlock.
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
    cancel: &AtomicBool,
) -> Option<PermitPair> {
    let other = other.filter(|other| other.key != one.key);
    let Some(other) = other else {
        return one.acquire(cancel).map(|first| PermitPair {
            first,
            second: None,
        });
    };
    let (low, high) = if one.key <= other.key {
        (one, other)
    } else {
        (other, one)
    };
    let first = low.acquire(cancel)?;
    let second = high.acquire(cancel)?;
    Some(PermitPair {
        first,
        second: Some(second),
    })
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

/// Classifies an error as a congestion signal (the peer asks to slow down)
/// or an ordinary failure. Backends report rate limits and "too many
/// requests/connections" as `ErrorKind::QuotaExceeded`; the textual checks
/// cover peers and older components that only send a message.
pub fn classify_error(error: &std::io::Error) -> OpOutcome {
    use std::io::ErrorKind;
    if matches!(
        error.kind(),
        ErrorKind::QuotaExceeded | ErrorKind::TimedOut | ErrorKind::WouldBlock
    ) {
        return OpOutcome::Overload;
    }
    let text = error.to_string().to_ascii_lowercase();
    const SIGNALS: [&str; 10] = [
        "too many concurrent",
        "too many requests",
        "too many connections",
        "ratelimitexceeded",
        "rate limit exceeded",
        "http 429",
        "http 503",
        "status 429",
        "status 503",
        "timed out",
    ];
    if SIGNALS.iter().any(|signal| text.contains(signal)) {
        OpOutcome::Overload
    } else {
        OpOutcome::Failed
    }
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
