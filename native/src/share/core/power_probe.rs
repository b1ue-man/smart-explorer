//! Connection probes requested by the host after a wake alarm or a network
//! change. Every running signal worker answers its share of a probe; the
//! ticket returns once all have answered or after `PROBE_WAIT`.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Longest a ticket waits (API `share.wake`: at most 12 s).
pub(crate) const PROBE_WAIT: Duration = Duration::from_secs(12);

/// Result of a probe: `ok` when the Share host is reachable as configured
/// (signal connection verified, or no server configured), `reconnected` when
/// a new signal connection had to be established for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub ok: bool,
    pub reconnected: bool,
}

struct ProbeState {
    remaining: usize,
    outcome: ProbeOutcome,
}

struct ProbeShared {
    state: Mutex<ProbeState>,
    answered: Condvar,
}

impl ProbeShared {
    fn lock(&self) -> MutexGuard<'_, ProbeState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Handle of one `request_probe` call.
pub struct ProbeTicket {
    shared: Arc<ProbeShared>,
}

impl ProbeTicket {
    /// Blocks until every running signal worker answered, at most 12 s.
    pub fn wait(self) -> ProbeOutcome {
        self.wait_timeout(PROBE_WAIT)
    }

    /// Like [`ProbeTicket::wait`] with a caller-chosen bound.
    pub fn wait_timeout(self, timeout: Duration) -> ProbeOutcome {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        while state.remaining > 0 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            state = match self.shared.answered.wait_timeout(state, deadline - now) {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        state.outcome
    }

    /// Whether every worker has answered already.
    pub fn is_answered(&self) -> bool {
        self.shared.lock().remaining == 0
    }
}

/// One worker's share of a probe. Dropping it unanswered (worker stopped)
/// counts as a failed answer, so a ticket never waits for a gone worker.
pub(crate) struct ProbeResponder {
    shared: Arc<ProbeShared>,
    answered: bool,
}

impl ProbeResponder {
    fn answer(&mut self, outcome: ProbeOutcome) {
        if std::mem::replace(&mut self.answered, true) {
            return;
        }
        let mut state = self.shared.lock();
        state.remaining = state.remaining.saturating_sub(1);
        state.outcome.ok |= outcome.ok;
        state.outcome.reconnected |= outcome.reconnected;
        drop(state);
        self.shared.answered.notify_all();
    }
}

impl Drop for ProbeResponder {
    fn drop(&mut self) {
        self.answer(ProbeOutcome::default());
    }
}

/// A ticket and one responder per subscribed worker. Without workers the
/// ticket is answered at once with a failed outcome: no Share host runs.
pub(crate) fn new_probe(workers: usize) -> (ProbeTicket, Vec<ProbeResponder>) {
    let shared = Arc::new(ProbeShared {
        state: Mutex::new(ProbeState {
            remaining: workers,
            outcome: ProbeOutcome::default(),
        }),
        answered: Condvar::new(),
    });
    let responders = (0..workers)
        .map(|_| ProbeResponder {
            shared: shared.clone(),
            answered: false,
        })
        .collect();
    (ProbeTicket { shared }, responders)
}

/// Probes a worker has taken but not answered yet; later probes merge in.
#[derive(Default)]
pub(crate) struct ProbeBatch {
    pub(crate) network_changed: bool,
    responders: Vec<ProbeResponder>,
}

impl ProbeBatch {
    pub(crate) fn push(&mut self, responder: ProbeResponder, network_changed: bool) {
        self.network_changed |= network_changed;
        self.responders.push(responder);
    }

    pub(crate) fn merge(&mut self, other: ProbeBatch) {
        self.network_changed |= other.network_changed;
        self.responders.extend(other.responders);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.responders.is_empty()
    }

    pub(crate) fn answer(self, outcome: ProbeOutcome) {
        for mut responder in self.responders {
            responder.answer(outcome);
        }
    }
}
