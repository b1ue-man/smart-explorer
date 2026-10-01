use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crossbeam_channel::Sender;

use super::clock::{Now, PowerClock};
use super::probe::{new_probe, ProbeBatch, ProbeTicket};
use super::SignalPowerStatus;

/// At most one hold per this interval unless a request reaches further.
pub(crate) const HOLD_THROTTLE_MS: i64 = 5_000;
/// Longest hold the client requests; a recorded hold end further away than
/// this means the wall clock went backwards.
const MAX_HOLD_MS: i64 = 60_000;

/// Process power state: one global instance in production, private
/// instances with a manual clock in tests.
pub(crate) struct PowerHub {
    clock: Arc<dyn PowerClock>,
    low_power: AtomicBool,
    state: Mutex<HubState>,
}

#[derive(Default)]
struct HubState {
    next_subscriber: u64,
    subscribers: Vec<Subscriber>,
    hook: Option<fn(u32)>,
    hold_until_ms: i64,
    last_hold_ms: Option<i64>,
    status: SignalPowerStatus,
    status_owner: Option<u64>,
}

struct Subscriber {
    id: u64,
    wake: Sender<()>,
    probe: ProbeBatch,
}

impl PowerHub {
    pub(crate) fn new(clock: Arc<dyn PowerClock>) -> Self {
        Self {
            clock,
            low_power: AtomicBool::new(false),
            state: Mutex::new(HubState::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn clock(&self) -> &Arc<dyn PowerClock> {
        &self.clock
    }

    pub(crate) fn now(&self) -> Now {
        self.clock.now()
    }

    pub(crate) fn set_low_power(&self, on: bool) {
        if self.low_power.swap(on, Ordering::AcqRel) != on {
            self.wake_all();
        }
    }

    pub(crate) fn low_power(&self) -> bool {
        self.low_power.load(Ordering::Acquire)
    }

    pub(crate) fn request_probe(&self, network_changed: bool) -> ProbeTicket {
        let mut state = self.lock();
        let (ticket, responders) = new_probe(state.subscribers.len());
        for (subscriber, responder) in state.subscribers.iter_mut().zip(responders) {
            subscriber.probe.push(responder, network_changed);
            let _ = subscriber.wake.try_send(());
        }
        ticket
    }

    pub(crate) fn set_activity_hook(&self, hook: fn(u32)) {
        self.lock().hook = Some(hook);
    }

    /// Asks the host to keep the CPU awake for `hold_ms`; only in low-power
    /// operation and only when a host registered a hook.
    pub(crate) fn request_hold(&self, hold_ms: u32) {
        if hold_ms == 0 || !self.low_power() {
            return;
        }
        let now_ms = self.clock.now().wall_ms;
        let hook = {
            let mut state = self.lock();
            let Some(hook) = state.hook else {
                return;
            };
            let end_ms = now_ms.saturating_add(i64::from(hold_ms));
            if state.hold_until_ms > now_ms.saturating_add(MAX_HOLD_MS) {
                state.hold_until_ms = 0;
                state.last_hold_ms = None;
            }
            if !hold_needed(state.hold_until_ms, state.last_hold_ms, now_ms, end_ms) {
                return;
            }
            state.hold_until_ms = end_ms;
            state.last_hold_ms = Some(now_ms);
            hook
        };
        // Outside the lock: the host may call back into this module.
        hook(hold_ms);
    }

    pub(crate) fn signal_status(&self) -> SignalPowerStatus {
        self.lock().status.clone()
    }

    pub(crate) fn subscribe(self: &Arc<Self>, wake: Sender<()>) -> PowerSubscription {
        let mut state = self.lock();
        let id = state.next_subscriber;
        state.next_subscriber = state.next_subscriber.wrapping_add(1);
        state.subscribers.push(Subscriber {
            id,
            wake,
            probe: ProbeBatch::default(),
        });
        PowerSubscription {
            hub: self.clone(),
            id,
        }
    }

    fn wake_all(&self) {
        for subscriber in &self.lock().subscribers {
            let _ = subscriber.wake.try_send(());
        }
    }
}

/// Covered requests and requests within the throttle interval that extend
/// the current hold by at most the interval are dropped; a longer request
/// always wins.
pub(crate) fn hold_needed(
    hold_until_ms: i64,
    last_hold_ms: Option<i64>,
    now_ms: i64,
    end_ms: i64,
) -> bool {
    if end_ms <= hold_until_ms {
        return false;
    }
    !last_hold_ms.is_some_and(|last| {
        now_ms.saturating_sub(last) < HOLD_THROTTLE_MS
            && end_ms.saturating_sub(hold_until_ms) <= HOLD_THROTTLE_MS
    })
}

/// A signal worker's registration: power changes and probes wake it.
pub(crate) struct PowerSubscription {
    hub: Arc<PowerHub>,
    id: u64,
}

impl PowerSubscription {
    pub(crate) fn hub(&self) -> &Arc<PowerHub> {
        &self.hub
    }

    pub(crate) fn take_probe(&self) -> Option<ProbeBatch> {
        let mut state = self.hub.lock();
        let subscriber = state
            .subscribers
            .iter_mut()
            .find(|subscriber| subscriber.id == self.id)?;
        let probe = std::mem::take(&mut subscriber.probe);
        (!probe.is_empty()).then_some(probe)
    }

    /// Makes this worker the source of `signal_power_status` (latest
    /// connection wins) and resets the status to `status`.
    pub(crate) fn claim_status(&self, status: SignalPowerStatus) {
        let mut state = self.hub.lock();
        state.status_owner = Some(self.id);
        state.status = status;
    }

    /// Updates the status while this worker owns it.
    pub(crate) fn update_status(&self, update: impl FnOnce(&mut SignalPowerStatus)) {
        let mut state = self.hub.lock();
        if state.status_owner == Some(self.id) {
            update(&mut state.status);
        }
    }
}

impl Drop for PowerSubscription {
    fn drop(&mut self) {
        let mut state = self.hub.lock();
        // Unanswered probes of this worker drop here and answer as failed.
        state
            .subscribers
            .retain(|subscriber| subscriber.id != self.id);
        if state.status_owner == Some(self.id) {
            state.status_owner = None;
            state.status = SignalPowerStatus::default();
        }
    }
}
