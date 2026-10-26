//! One bounded picture/control allowance per authenticated peer, shared by windows.
use super::pacing::guard_bytes;
use super::rate_control::PathKind;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

const CONTROL_RESERVE: u64 = 256 * 1024;
static PEERS: OnceLock<Mutex<HashMap<String, Weak<Mutex<Budget>>>>> = OnceLock::new();
#[derive(Default)]
struct Flow {
    requested_us: u64,
    estimate: u64,
    reserved: u64,
    frames: Vec<(u64, u64, u64)>,
}
#[derive(Default)]
struct Budget {
    control_bytes: u64,
    next: u64,
    flows: HashMap<u64, Flow>,
    turn: u64,
}
/// The lease owns exactly one session's debt; resetting it never releases a sibling's debt.
pub(crate) struct SenderBudget {
    shared: Arc<Mutex<Budget>>,
    id: u64,
}
pub(crate) struct EncodePermit {
    shared: Arc<Mutex<Budget>>,
    id: u64,
    committed: bool,
}
impl SenderBudget {
    pub(crate) fn for_peer(peer: String) -> Self {
        let registry = PEERS.get_or_init(Mutex::default);
        let mut peers = registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        peers.retain(|_, budget| budget.strong_count() > 0);
        let shared = peers.get(&peer).and_then(Weak::upgrade).unwrap_or_else(|| {
            let value = Arc::new(Mutex::new(Budget::default()));
            peers.insert(peer, Arc::downgrade(&value));
            value
        });
        let id = {
            let mut state = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.next = state.next.saturating_add(1);
            let id = state.next;
            state.flows.insert(id, Flow::default());
            id
        };
        Self { shared, id }
    }
    pub(crate) fn admit(
        &self,
        bps: u64,
        path: PathKind,
        rtt_us: u64,
        now_us: u64,
        keyframe: bool,
    ) -> Option<EncodePermit> {
        let mut state = self.shared.lock().ok()?;
        state.flows.get_mut(&self.id)?.requested_us = now_us;
        let picture_cap = guard_bytes(bps, path, rtt_us).max(1);
        let total: u64 = state
            .flows
            .values()
            .map(|flow| {
                flow.reserved
                    .saturating_add(flow.frames.iter().map(|f| f.2).sum::<u64>())
            })
            .sum();
        let count = state.flows.len().max(1) as u64;
        let (estimate, own) = {
            let flow = state.flows.get(&self.id)?;
            (
                flow.estimate,
                flow.reserved
                    .saturating_add(flow.frames.iter().map(|f| f.2).sum::<u64>()),
            )
        };
        let estimate = if keyframe || estimate == 0 {
            if total != 0 {
                return None;
            }
            picture_cap
        } else {
            estimate
        };
        if own != 0 && (own >= picture_cap / count || total.saturating_add(estimate) > picture_cap)
        {
            return None;
        }
        // One larger reference may stream through an empty budget. It never opens a second queue.
        if total != 0 && total.saturating_add(estimate) > picture_cap {
            return None;
        }
        if total == 0
            && count > 1
            && state.turn != 0
            && state.turn != self.id
            && state.flows.get(&state.turn).is_some_and(|flow| {
                flow.requested_us != 0 && now_us.saturating_sub(flow.requested_us) <= 1_000_000
            })
        {
            return None;
        }
        state.flows.get_mut(&self.id)?.reserved = estimate;
        let mut ids: Vec<u64> = state.flows.keys().copied().collect();
        ids.sort_unstable();
        state.turn = ids
            .iter()
            .copied()
            .find(|id| *id > self.id)
            .unwrap_or(ids[0]);
        Some(EncodePermit {
            shared: self.shared.clone(),
            id: self.id,
            committed: false,
        })
    }
    pub(crate) fn ack(&self, gop: u64, frame: u64) {
        if let Ok(mut state) = self.shared.lock() {
            if let Some(flow) = state.flows.get_mut(&self.id) {
                flow.frames.retain(|(g, f, _)| *g != gop || *f > frame);
            }
        }
    }
    pub(crate) fn reset(&self) {
        if let Ok(mut state) = self.shared.lock() {
            if let Some(flow) = state.flows.get_mut(&self.id) {
                flow.frames.clear();
                flow.reserved = 0;
            }
        }
    }
    /// Control is a reserved share of the same relay ceiling, before any picture allowance.
    pub(crate) fn control_reserve_bytes() -> u64 {
        CONTROL_RESERVE
    }
}
impl EncodePermit {
    pub(crate) fn sent(mut self, gop: u64, frame: u64, bytes: u64) {
        if let Ok(mut state) = self.shared.lock() {
            if let Some(flow) = state.flows.get_mut(&self.id) {
                flow.reserved = 0;
                flow.estimate = bytes;
                flow.frames.push((gop, frame, bytes));
            }
        }
        self.committed = true;
    }
}
impl Drop for EncodePermit {
    fn drop(&mut self) {
        if !self.committed {
            if let Ok(mut state) = self.shared.lock() {
                if let Some(flow) = state.flows.get_mut(&self.id) {
                    flow.reserved = 0;
                }
            }
        }
    }
}
impl Drop for SenderBudget {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.lock() {
            state.flows.remove(&self.id);
            if state.turn == self.id {
                state.turn = 0;
            }
        }
    }
}

/// A control write has a separately reserved part of the same per-peer ceiling.
pub(crate) struct ControlPermit {
    shared: Arc<Mutex<Budget>>,
    bytes: u64,
}
pub(crate) fn reserve_control(peer: String, bytes: usize) -> Option<ControlPermit> {
    let lease = SenderBudget::for_peer(peer);
    let bytes = u64::try_from(bytes).ok()?;
    {
        let mut state = lease.shared.lock().ok()?;
        let total = state.control_bytes.checked_add(bytes)?;
        if total > CONTROL_RESERVE {
            return None;
        }
        state.control_bytes = total;
    }
    Some(ControlPermit {
        shared: lease.shared.clone(),
        bytes,
    })
}
impl Drop for ControlPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.lock() {
            state.control_bytes = state.control_bytes.saturating_sub(self.bytes);
        }
    }
}

#[cfg(test)]
#[path = "batches_sender_task_tests.rs"]
mod batches_task_tests;
