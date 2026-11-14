//! Global, source and network limits of the client registrations (E-v2-1, R2).
//!
//! A registration that a reached limit refuses, and that no older client may
//! displace, is refused before anything is bound: it binds no device id, takes no
//! token of the new-binding rate limiter and writes no copy of the bindings.

use super::{RegistrationError, State};
use crate::limits::SourceKey;
use crate::Writer;

/// Which of the global, source and network limits a new client of one source reaches.
pub(super) struct Capacity {
    pub(super) full: bool,
    pub(super) source_full: bool,
    pub(super) network_full: bool,
}

impl Capacity {
    pub(super) fn of(state: &State, source: SourceKey) -> Self {
        let limits = state.policy.limits;
        let full = state.clients.len() >= limits.max_clients;
        let source_full = source.has_internal_source_limit()
            && state
                .clients
                .values()
                .filter(|client| client.source == source)
                .count()
                >= limits.max_clients_per_source;
        let network_full = source.network().is_some_and(|network| {
            state
                .clients
                .values()
                .filter(|client| client.source.network() == Some(network))
                .count()
                >= limits.max_clients_per_network
        });
        Self {
            full,
            source_full,
            network_full,
        }
    }

    /// Whether one of the three limits is reached.
    pub(super) fn any(&self) -> bool {
        self.full || self.source_full || self.network_full
    }

    /// The refusal without a displacement: `Full` for the global limit, else `SourceFull`.
    pub(super) fn refusal(&self) -> RegistrationError {
        if self.full {
            RegistrationError::Full
        } else {
            RegistrationError::SourceFull
        }
    }
}

/// The older client that a proven registration displaces at a reached limit: one
/// without key login of the scope that the limit applies to, one without
/// subscriptions first, then the oldest.
pub(super) fn displacement(
    state: &State,
    source: SourceKey,
    capacity: &Capacity,
) -> Option<(u64, Writer)> {
    eviction_candidate(
        state,
        (!capacity.full).then_some(source),
        capacity.network_full && !capacity.source_full,
    )
}

/// Whether a proven registration of `device_id` at a reached limit can displace a
/// client: an unproven client of the same device id, or a [`displacement`].
pub(super) fn can_displace(
    state: &State,
    device_id: &str,
    source: SourceKey,
    capacity: &Capacity,
) -> bool {
    state
        .clients
        .values()
        .any(|client| client.device_id == device_id && client.identity.proven().is_none())
        || displacement(state, source, capacity).is_some()
}

/// An older client without key login, of `source` when given: one without
/// subscriptions first, then the oldest.
fn eviction_candidate(
    state: &State,
    source: Option<SourceKey>,
    network: bool,
) -> Option<(u64, Writer)> {
    state
        .clients
        .iter()
        .filter(|(_, client)| client.identity.proven().is_none())
        .filter(|(_, client)| {
            source.is_none_or(|source| {
                if network {
                    client.source.network() == source.network()
                } else {
                    client.source == source
                }
            })
        })
        .min_by_key(|(id, client)| {
            let subscribed = !client.direct_lookup_ids.is_empty()
                || !client.watched_lookup_ids.is_empty()
                || !client.rooms.is_empty();
            (subscribed, **id)
        })
        .map(|(id, client)| (*id, client.writer.clone()))
}
