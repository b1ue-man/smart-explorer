//! Slots of the TLS handshakes this host runs before it knows any identity
//! (R-ZUGANG-01, review 2026-11-07; rest of S07 of the review 2026-10-02).
//!
//! The QUIC Initial of an unknown sender may name any source address, and a
//! held handshake costs 3 s (`node_accept.rs`). Three rules keep these slots
//! for paired devices under load:
//!
//! * At most a quarter of the slots per source: one IPv4 address, one IPv6
//!   /64 prefix (one subscriber line), or one EndpointId that a relay named
//!   (`IncomingAddr::Relay`). A quarter, not one, so several devices behind
//!   one mobile carrier NAT still connect at once.
//! * Once half of the slots are taken, a direct sender whose address is not
//!   validated gets a QUIC Retry instead of a slot (RFC 9000 section 8.1;
//!   iroh's `Router` does the same in `IncomingFilterOutcome::Retry`). A
//!   forged sender never answers it; a real one pays one round trip, only
//!   under this load.
//! * A quarter of the slots is reserved for relay connections whose
//!   EndpointId the relay names before the handshake and that this host
//!   already pins (`ShareIrohNode::known_endpoint`).
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use iroh::endpoint::IncomingAddr;

/// Where one incoming handshake comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::share) enum TlsSource {
    /// An IPv4 address (also IPv4-mapped IPv6).
    V4(u32),
    /// The /64 prefix of an IPv6 address.
    V6(u64),
    /// The EndpointId a relay names for the sender.
    Relay(String),
    /// Any other transport, by its address text.
    Other(String),
}

impl TlsSource {
    pub(in crate::share) fn of(addr: &IncomingAddr) -> Self {
        match addr {
            IncomingAddr::Ip(socket) => Self::of_ip(socket.ip()),
            IncomingAddr::Relay { endpoint_id, .. } => Self::Relay(endpoint_id.to_string()),
            other => Self::Other(format!("{other:?}")),
        }
    }

    pub(in crate::share) fn of_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => Self::V4(u32::from(v4)),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => Self::V4(u32::from(v4)),
                None => Self::V6((u128::from(v6) >> 64) as u64),
            },
        }
    }

    /// Whether the sender's address can be forged (direct UDP): only such a
    /// sender is asked for address validation.
    fn forgeable(&self) -> bool {
        matches!(self, Self::V4(_) | Self::V6(_))
    }
}

/// The decision for one incoming handshake.
pub(in crate::share) enum TlsAdmission {
    /// Run the handshake while holding this slot.
    Slot(TlsSlot),
    /// Answer with a QUIC Retry; no slot is taken.
    Retry,
    /// Refuse the attempt.
    Refuse,
}

#[derive(Default)]
struct TlsState {
    total: usize,
    sources: HashMap<TlsSource, usize>,
}

pub(in crate::share) struct TlsHandshakeSlots {
    capacity: usize,
    /// Slots only pinned relay senders may take.
    reserve: usize,
    per_source: usize,
    /// Taken slots from which unvalidated direct senders get a Retry.
    retry_from: usize,
    state: Mutex<TlsState>,
}

impl TlsHandshakeSlots {
    pub(in crate::share) fn new(capacity: usize) -> Arc<Self> {
        let capacity = capacity.max(1);
        Arc::new(Self {
            capacity,
            reserve: capacity / 4,
            per_source: (capacity / 4).max(1),
            retry_from: (capacity / 2).max(1),
            state: Mutex::new(TlsState::default()),
        })
    }

    /// Decides one incoming handshake from `source`. `validated`: QUIC has
    /// validated the sender's address; `pinned`: a relay named a sender this
    /// host pins.
    pub(in crate::share) fn admit(
        self: &Arc<Self>,
        source: TlsSource,
        validated: bool,
        pinned: bool,
    ) -> TlsAdmission {
        let mut state = self.lock();
        if source.forgeable() && !validated && state.total >= self.retry_from {
            return TlsAdmission::Retry;
        }
        let limit = if pinned && matches!(source, TlsSource::Relay(_)) {
            self.capacity
        } else {
            self.capacity - self.reserve
        };
        if state.total >= limit {
            return TlsAdmission::Refuse;
        }
        let held = state.sources.get(&source).copied().unwrap_or(0);
        if held >= self.per_source {
            return TlsAdmission::Refuse;
        }
        state.total += 1;
        state.sources.insert(source.clone(), held + 1);
        TlsAdmission::Slot(TlsSlot {
            slots: self.clone(),
            source,
        })
    }

    fn lock(&self) -> MutexGuard<'_, TlsState> {
        // The counters stay consistent under a panic elsewhere: every change
        // is one complete update under the lock.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn release(&self, source: &TlsSource) {
        let mut state = self.lock();
        state.total = state.total.saturating_sub(1);
        if let Some(held) = state.sources.get_mut(source) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                state.sources.remove(source);
            }
        }
    }

    /// Taken slots in total and of `source`.
    #[cfg(test)]
    pub(in crate::share) fn held(&self, source: &TlsSource) -> (usize, usize) {
        let state = self.lock();
        (state.total, state.sources.get(source).copied().unwrap_or(0))
    }
}

/// One taken handshake slot; dropping it frees the slot.
pub(in crate::share) struct TlsSlot {
    slots: Arc<TlsHandshakeSlots>,
    source: TlsSource,
}

impl Drop for TlsSlot {
    fn drop(&mut self) {
        self.slots.release(&self.source);
    }
}

#[cfg(test)]
#[path = "handshake_limits_tls_tests.rs"]
mod tests;
