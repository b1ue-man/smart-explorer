use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use iroh_base::PublicKey;

use super::bindings::{BindOutcome, Bindings};
use super::discovery_state::unix_seconds;
use super::limits::{validate_identifier, ServerLimits, SourceKey};
use super::relay_access::RelayAdmissions;
use super::{send, Out, PeerPresence, Writer};

#[cfg(test)]
pub(super) use super::rooms::join_room;
pub(super) use super::rooms::leave_room;

#[derive(Clone)]
pub(super) struct Client {
    pub(super) writer: Writer,
    pub(super) source: SourceKey,
    pub(super) device_id: String,
    pub(super) capabilities: HashSet<String>,
    pub(super) direct_lookup_ids: HashSet<String>,
    pub(super) watched_lookup_ids: HashSet<String>,
    pub(super) rooms: HashSet<String>,
    pub(super) identity: ClientIdentity,
}

/// What a client proved at its Hello (FC4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum ClientIdentity {
    /// No key login (an older app); may use unbound entries only.
    #[default]
    Legacy,
    /// An older app with the key its Hello names (unproven; the relay
    /// handshake proves it before the relay admits it).
    LegacyClaimed(PublicKey),
    /// Logged in with this key (`key_login_v1`).
    Proven(PublicKey),
}

impl ClientIdentity {
    pub(super) fn proven(&self) -> Option<&PublicKey> {
        match self {
            Self::Proven(key) => Some(key),
            Self::Legacy | Self::LegacyClaimed(_) => None,
        }
    }

    fn relay_key(&self) -> Option<PublicKey> {
        match self {
            Self::Proven(key) | Self::LegacyClaimed(key) => Some(*key),
            Self::Legacy => None,
        }
    }
}

/// Server-wide settings the handlers read (FC4).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ServerPolicy {
    pub(super) limits: ServerLimits,
    /// Older clients without key login are refused (`--require-key-login`).
    pub(super) require_key_login: bool,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) next_id: u64,
    pub(super) next_discovery_id: u64,
    pub(super) clients: HashMap<u64, Client>,
    pub(super) direct: HashMap<String, (u64, PeerPresence)>,
    pub(super) watchers: HashMap<String, HashSet<u64>>,
    pub(super) rooms: HashMap<String, HashMap<String, (u64, PeerPresence)>>,
    pub(super) discovery_offers: HashMap<String, super::discovery_state::DiscoveryOffer>,
    pub(super) discovery_offer_index: HashMap<(u64, String), String>,
    pub(super) discovery_exchanges: HashMap<String, super::discovery_state::DiscoveryExchange>,
    pub(super) policy: ServerPolicy,
    pub(super) bindings: Bindings,
    pub(super) binding_path: Option<std::path::PathBuf>,
    pub(super) relay_admissions: Arc<RelayAdmissions>,
    /// Proof hash a proven watcher showed, by (client id, lookup id).
    pub(super) watch_access: HashMap<(u64, String), [u8; 32]>,
    /// Partition of a room member, by (room id, device id).
    pub(super) room_access: HashMap<(String, String), [u8; 32]>,
}

impl State {
    /// Called while the state lock serializes owner changes and their disk order.
    pub(super) fn persist_bindings(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.binding_path else { return Ok(()); };
        let mut candidate = self.bindings.clone();
        if candidate.take_dirty().is_none() { return Ok(()); }
        candidate.save(path)?;
        self.bindings = candidate;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RegistrationError {
    Full,
    SourceFull,
    InvalidDeviceId,
    IdExhausted,
    /// The device id belongs to another proven key.
    DeviceBound,
    /// The proven key already has its maximum of registrations.
    KeyFull,
    /// The server accepts only clients that log in with their key.
    KeyLoginRequired,
    BindingStateUnavailable,
}

impl RegistrationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Full => "server client limit reached",
            Self::SourceFull => "server source client limit reached",
            Self::InvalidDeviceId => "invalid or oversized device id",
            Self::IdExhausted => "server client id space exhausted",
            Self::DeviceBound => "device id is bound to another device key",
            Self::KeyFull => "too many connections for this device key",
            Self::BindingStateUnavailable => "cannot persist device key binding; check server state file",
            Self::KeyLoginRequired => "this server requires key login; update Smart Explorer",
        }
    }
}

enum Registration {
    Done(u64),
    /// Displaced clients go first; then the registration is tried again.
    Retry,
    Failed(RegistrationError),
}

pub(super) fn lock_state(state: &Arc<Mutex<State>>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Registration of a client without key login.
pub(super) fn register_client(
    state: &Arc<Mutex<State>>,
    writer: Writer,
    source: SourceKey,
    device_id: String,
    capabilities: HashSet<String>,
) -> Result<u64, RegistrationError> {
    register_client_with_identity(
        state,
        writer,
        source,
        device_id,
        capabilities,
        ClientIdentity::Legacy,
    )
}

/// Registers a client. A proven key binds the device id (first key wins);
/// older registrations that only named it are closed. When the server is
/// full, a proven client displaces an older client without key login,
/// preferring one without subscriptions (FC4).
pub(super) fn register_client_with_identity(
    state: &Arc<Mutex<State>>,
    writer: Writer,
    source: SourceKey,
    device_id: String,
    capabilities: HashSet<String>,
    identity: ClientIdentity,
) -> Result<u64, RegistrationError> {
    if validate_identifier("device id", &device_id).is_err() {
        return Err(RegistrationError::InvalidDeviceId);
    }
    // At most three evictions: an impostor, a global slot, then a source/network slot.
    for _ in 0..4 {
        let (registration, displaced) = {
            let mut state = lock_state(state);
            try_register_locked(
                &mut state,
                &writer,
                source,
                &device_id,
                &capabilities,
                &identity,
            )
        };
        for (client_id, displaced_writer) in displaced {
            displaced_writer.close();
            cleanup(client_id, state);
        }
        match registration {
            Registration::Done(id) => return Ok(id),
            Registration::Failed(error) => return Err(error),
            Registration::Retry => {}
        }
    }
    Err(RegistrationError::Full)
}

fn try_register_locked(
    state: &mut State,
    writer: &Writer,
    source: SourceKey,
    device_id: &str,
    capabilities: &HashSet<String>,
    identity: &ClientIdentity,
) -> (Registration, Vec<(u64, Writer)>) {
    let limits = state.policy.limits;
    match identity.proven() {
        Some(key) => {
            let bound = state
                .bindings
                .bind_device(device_id, &key.to_string(), unix_seconds());
            if bound == BindOutcome::Conflict {
                return (Registration::Failed(RegistrationError::DeviceBound), Vec::new());
            }
            if bound == BindOutcome::Full || state.persist_bindings().is_err() {
                return (Registration::Failed(RegistrationError::BindingStateUnavailable), Vec::new());
            }
            let impostors: Vec<(u64, Writer)> = state
                .clients
                .iter()
                .filter(|(_, client)| {
                    client.device_id == device_id && client.identity.proven().is_none()
                })
                .map(|(id, client)| (*id, client.writer.clone()))
                .collect();
            if !impostors.is_empty() {
                return (Registration::Retry, impostors);
            }
            let same_key = state
                .clients
                .values()
                .filter(|client| client.identity.proven() == Some(key))
                .count();
            if same_key >= limits.max_clients_per_key {
                return (Registration::Failed(RegistrationError::KeyFull), Vec::new());
            }
        }
        None => {
            if state.policy.require_key_login {
                return (
                    Registration::Failed(RegistrationError::KeyLoginRequired),
                    Vec::new(),
                );
            }
            if state.bindings.device_key(device_id).is_some() {
                return (Registration::Failed(RegistrationError::DeviceBound), Vec::new());
            }
        }
    }
    let full = state.clients.len() >= limits.max_clients;
    let source_full = source.has_internal_source_limit()
        && state
            .clients
            .values()
            .filter(|client| client.source == source)
            .count()
            >= limits.max_clients_per_source;
    let network_full = source.network().is_some_and(|network| state.clients.values()
        .filter(|client| client.source.network() == Some(network)).count() >= limits.max_clients_per_network);
    if full || source_full || network_full {
        let victim = identity
            .proven()
            .and_then(|_| eviction_candidate(state, (!full).then_some(source), network_full && !source_full));
        return match victim {
            Some(victim) => (Registration::Retry, vec![victim]),
            None if full => (Registration::Failed(RegistrationError::Full), Vec::new()),
            None => (Registration::Failed(RegistrationError::SourceFull), Vec::new()),
        };
    }
    let Some(id) = state.next_id.checked_add(1) else {
        return (Registration::Failed(RegistrationError::IdExhausted), Vec::new());
    };
    state.next_id = id;
    state.clients.insert(
        id,
        Client {
            writer: writer.clone(),
            source,
            device_id: device_id.to_string(),
            capabilities: capabilities.clone(),
            direct_lookup_ids: HashSet::new(),
            watched_lookup_ids: HashSet::new(),
            rooms: HashSet::new(),
            identity: identity.clone(),
        },
    );
    if let Some(key) = identity.relay_key() {
        state.relay_admissions.signed_in(key);
    }
    (Registration::Done(id), Vec::new())
}

/// An older client without key login, of `source` when given: one without
/// subscriptions first, then the oldest.
fn eviction_candidate(state: &State, source: Option<SourceKey>, network: bool) -> Option<(u64, Writer)> {
    state
        .clients
        .iter()
        .filter(|(_, client)| client.identity.proven().is_none())
        .filter(|(_, client)| source.is_none_or(|source| {
            if network { client.source.network() == source.network() } else { client.source == source }
        }))
        .min_by_key(|(id, client)| {
            let subscribed = !client.direct_lookup_ids.is_empty()
                || !client.watched_lookup_ids.is_empty()
                || !client.rooms.is_empty();
            (subscribed, **id)
        })
        .map(|(id, client)| (*id, client.writer.clone()))
}

/// Whether a watcher may see a lookup's presence: older clients always (they
/// cannot show proofs), proven ones with the owner's access hash.
pub(super) fn watcher_admitted(state: &State, watcher_id: u64, lookup_id: &str) -> bool {
    let Some(client) = state.clients.get(&watcher_id) else {
        return false;
    };
    if client.identity.proven().is_none() {
        return true;
    }
    match state.bindings.lookup_access(lookup_id) {
        Some(hash) => state
            .watch_access
            .get(&(watcher_id, lookup_id.to_string()))
            .is_some_and(|shown| *shown == hash),
        None => true,
    }
}

pub(super) fn cleanup(id: u64, state: &Arc<Mutex<State>>) {
    let notifications = {
        let mut state = lock_state(state);
        let Some(client) = state.clients.remove(&id) else {
            return;
        };
        if let Some(key) = client.identity.relay_key() {
            state.relay_admissions.signed_out(key);
        }
        let mut notifications = super::discovery::cleanup_client_locked(&mut state, id);

        for lookup_id in client.direct_lookup_ids {
            if state.direct.get(&lookup_id).map(|(owner, _)| *owner) == Some(id) {
                state.direct.remove(&lookup_id);
                if let Some(watchers) = state.watchers.get(&lookup_id) {
                    for watcher_id in watchers {
                        if !watcher_admitted(&state, *watcher_id, &lookup_id) {
                            continue;
                        }
                        if let Some(watcher) = state.clients.get(watcher_id) {
                            notifications.push((
                                watcher.writer.clone(),
                                Out::DirectOffline {
                                    lookup_id: lookup_id.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }

        for lookup_id in client.watched_lookup_ids {
            state.watch_access.remove(&(id, lookup_id.clone()));
            let remove_lookup = if let Some(watchers) = state.watchers.get_mut(&lookup_id) {
                watchers.remove(&id);
                watchers.is_empty()
            } else {
                false
            };
            if remove_lookup {
                state.watchers.remove(&lookup_id);
            }
        }

        for room_id in client.rooms {
            notifications.extend(super::rooms::depart_locked(&mut state, id, &room_id));
        }
        notifications
    };
    send_all(notifications);
}

pub(super) fn send_all(notifications: Vec<(Writer, Out)>) {
    for (writer, message) in notifications {
        send(&writer, &message);
    }
}
