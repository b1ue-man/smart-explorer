//! Places for signaling connections before their registration (R-SERVER-04).
//!
//! A connection holds a handshake place from its accept until its
//! registration (or its end). The pool is smaller than the worker pool, so
//! connections that never register cannot take the places of registered
//! clients, and a full pool closes its oldest connection of the full scope
//! (source, IPv6 network or whole pool) instead of refusing the newcomer:
//! an honest handshake finishes within a round trip or two, while a silent
//! connection waits for the ten-second registration deadline.
//!
//! The place travels with the connection thread (`hold` before the handler,
//! `release_current` at the registration), so the transport code needs no
//! extra parameter.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard};

use super::limits::{NetworkKey, ServerLimits, SourceKey};

/// Connections that wait for their registration, overall.
pub(super) const MAX_HANDSHAKES: usize = 128;
/// Connections that wait for their registration, per source. Several devices
/// behind one carrier-grade NAT rarely log in within the same ten seconds.
pub(super) const MAX_HANDSHAKES_PER_SOURCE: usize = 4;

struct Entry {
    source: SourceKey,
    /// A clone of the connection's socket; shutting it down ends the handler.
    stream: TcpStream,
}

#[derive(Default)]
struct Registry {
    next_id: u64,
    /// Ordered by id, that is by age.
    entries: BTreeMap<u64, Entry>,
}

#[derive(Clone)]
pub(super) struct HandshakeSlots {
    registry: Arc<Mutex<Registry>>,
    max_total: usize,
    max_per_source: usize,
    max_per_network: usize,
}

impl HandshakeSlots {
    pub(super) fn new(max_total: usize, max_per_source: usize, max_per_network: usize) -> Self {
        Self {
            registry: Arc::new(Mutex::new(Registry::default())),
            max_total: max_total.max(1),
            max_per_source: max_per_source.max(1),
            max_per_network: max_per_network.max(1),
        }
    }

    pub(super) fn from_limits(limits: &ServerLimits) -> Self {
        Self::new(
            limits.max_handshakes,
            limits.max_handshakes_per_source,
            limits.max_handshakes_per_network,
        )
    }

    /// A place for a new connection from `source`; a full scope first closes
    /// its oldest waiting connection. `None` only when the socket cannot be
    /// cloned for a later eviction.
    pub(super) fn try_acquire(
        &self,
        source: SourceKey,
        stream: &TcpStream,
    ) -> Option<HandshakeSlot> {
        let stream = stream.try_clone().ok()?;
        let mut registry = lock(&self.registry);
        // Each eviction frees one place of the full scope; at most one per scope.
        for _ in 0..3 {
            let Some(victim) = self.eviction_victim(&registry, source) else {
                break;
            };
            if let Some(entry) = registry.entries.remove(&victim) {
                let _ = entry.stream.shutdown(Shutdown::Both);
            }
        }
        let id = registry.next_id;
        registry.next_id = registry.next_id.wrapping_add(1);
        registry.entries.insert(id, Entry { source, stream });
        Some(HandshakeSlot {
            registry: self.registry.clone(),
            id,
        })
    }

    /// The oldest waiting connection of the narrowest full scope, if any.
    fn eviction_victim(&self, registry: &Registry, source: SourceKey) -> Option<u64> {
        let oldest = |matches: &dyn Fn(&Entry) -> bool| {
            registry
                .entries
                .iter()
                .find(|&(_, entry)| matches(entry))
                .map(|(id, _)| *id)
        };
        if source.has_internal_source_limit() {
            let same_source = |entry: &Entry| entry.source == source;
            if count(registry, &same_source) >= self.max_per_source {
                return oldest(&same_source);
            }
        }
        if let Some(network) = source.network() {
            let same_network = |entry: &Entry| in_network(entry, network);
            if count(registry, &same_network) >= self.max_per_network {
                return oldest(&same_network);
            }
        }
        if registry.entries.len() >= self.max_total {
            return oldest(&|_: &Entry| true);
        }
        None
    }
}

fn count(registry: &Registry, matches: &dyn Fn(&Entry) -> bool) -> usize {
    registry
        .entries
        .values()
        .filter(|&entry| matches(entry))
        .count()
}

fn in_network(entry: &Entry, network: NetworkKey) -> bool {
    entry.source.network() == Some(network)
}

fn lock(registry: &Mutex<Registry>) -> MutexGuard<'_, Registry> {
    registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One waiting connection's place; dropping it frees the place.
pub(super) struct HandshakeSlot {
    registry: Arc<Mutex<Registry>>,
    id: u64,
}

impl Drop for HandshakeSlot {
    fn drop(&mut self) {
        lock(&self.registry).entries.remove(&self.id);
    }
}

thread_local! {
    static CURRENT: RefCell<Option<HandshakeSlot>> = const { RefCell::new(None) };
}

/// Gives the connection thread its place.
pub(super) fn hold(slot: HandshakeSlot) {
    CURRENT.with(|current| *current.borrow_mut() = Some(slot));
}

/// Frees the connection thread's place (registered, or ended); a thread
/// without a place (tests, direct handler calls) does nothing.
pub(super) fn release_current() {
    let slot = CURRENT.with(|current| current.borrow_mut().take());
    drop(slot);
}

#[cfg(test)]
#[path = "handshake_slots_tests.rs"]
mod handshake_slots_tests;
