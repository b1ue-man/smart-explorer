use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use iroh_base::PublicKey;

use super::bindings::{persist_outside_lock, BindOutcome, Bindings, SaveOrder};
use super::discovery_state::unix_seconds;
use super::handshake_slots;
use super::limits::{validate_identifier, ServerLimits, SourceKey};
use super::rate_limits::NewBindingRateLimiter;
use super::relay_access::RelayAdmissions;
use super::{send, Out, PeerPresence, Writer};

#[path = "bindings_registration_error.rs"]
mod registration_error;
pub(super) use registration_error::RegistrationError;

#[path = "bindings_capacity.rs"]
mod capacity;
use capacity::{can_displace, displacement, Capacity};

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
    /// No key login (an older app); publishes no Direct presence and watches
    /// unbound lookups only (E60).
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

/// Server-wide settings the handlers read (FC4). The server sets them from
/// `config::Options`, where key login is required unless the operator gives
/// `--allow-legacy-clients` (E18). `Default` keeps the compatibility mode for
/// the unit tests that model older clients; they set `require_key_login`
/// where they test the refusal.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ServerPolicy {
    pub(super) limits: ServerLimits,
    /// Older clients without key login are refused (the default of the
    /// server; off only with `--allow-legacy-clients`).
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
    /// Orders the writes of the binding copies (SEC-SERVER-BESITZER-60S Teil 3).
    pub(super) binding_saves: Arc<SaveOrder>,
    pub(super) relay_admissions: Arc<RelayAdmissions>,
    /// Proof hash a proven watcher showed, by (client id, lookup id).
    pub(super) watch_access: HashMap<(u64, String), [u8; 32]>,
    /// Partition of a room member, by (room id, device id).
    pub(super) room_access: HashMap<(String, String), [u8; 32]>,
    /// New device/lookup bindings per source (R-SERVER-02).
    pub(super) binding_rate: NewBindingRateLimiter,
}

enum Registration {
    Done(u64),
    /// Displaced clients go first; then the registration is tried again.
    Retry,
    /// The bindings are saved outside the lock first; then the registration is tried again.
    Save,
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

/// At most three evictions per registration: an impostor, a global slot, then a
/// source or network slot.
const MAX_EVICTION_ROUNDS: usize = 3;
/// Saves one registration may wait for: its new binding, once (the next round finds it bound).
const MAX_SAVE_ROUNDS: usize = 1;

/// Registers a client. A proven key binds the device id (first key wins);
/// older registrations that only named it are closed. When the server is
/// full, a proven client displaces an older client without key login,
/// preferring one without subscriptions (FC4). The connection keeps its
/// handshake place until its registration is over (O-S5); a refused
/// registration gives the place back at once.
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
    handshake_slots::reserve_current();
    let result = register_rounds(state, &writer, source, &device_id, &capabilities, &identity);
    if result.is_err() {
        handshake_slots::release_current();
    }
    result
}

fn register_rounds(
    state: &Arc<Mutex<State>>,
    writer: &Writer,
    source: SourceKey,
    device_id: &str,
    capabilities: &HashSet<String>,
    identity: &ClientIdentity,
) -> Result<u64, RegistrationError> {
    let mut evictions = 0_usize;
    let mut saves = 0_usize;
    // The copy generation the binding must reach on disk; fixed by the first round
    // that binds, so a later change of another connection cannot force another save.
    let mut need: Option<u64> = None;
    loop {
        let (registration, displaced) = {
            let mut state = lock_state(state);
            try_register_locked(
                &mut state,
                writer,
                source,
                device_id,
                capabilities,
                identity,
                &mut need,
            )
        };
        for (client_id, displaced_writer) in displaced {
            displaced_writer.close();
            cleanup(client_id, state);
        }
        match registration {
            Registration::Done(id) => return Ok(id),
            Registration::Failed(error) => return Err(error),
            Registration::Retry => {
                evictions += 1;
                if evictions > MAX_EVICTION_ROUNDS {
                    return Err(RegistrationError::Full);
                }
            }
            Registration::Save => {
                saves += 1;
                if saves > MAX_SAVE_ROUNDS {
                    return Err(RegistrationError::Full);
                }
                if persist_outside_lock(state).is_err() {
                    return Err(RegistrationError::BindingStateUnavailable);
                }
            }
        }
    }
}

fn try_register_locked(
    state: &mut State,
    writer: &Writer,
    source: SourceKey,
    device_id: &str,
    capabilities: &HashSet<String>,
    identity: &ClientIdentity,
    need: &mut Option<u64>,
) -> (Registration, Vec<(u64, Writer)>) {
    let limits = state.policy.limits;
    match identity.proven() {
        Some(key) => {
            let key_text = key.to_string();
            // A device id bound to another key is refused before anything else.
            if state
                .bindings
                .device_key(device_id)
                .is_some_and(|bound| bound != key_text.as_str())
            {
                return (
                    Registration::Failed(RegistrationError::DeviceBound),
                    Vec::new(),
                );
            }
            // O-S3: the per-key connection limit is checked before a device id is bound.
            let same_key = state
                .clients
                .values()
                .filter(|client| client.identity.proven() == Some(key))
                .count();
            if same_key >= limits.max_clients_per_key {
                return (Registration::Failed(RegistrationError::KeyFull), Vec::new());
            }
            // R2 (E-v2-1): a registration that a reached global, source or network limit
            // refuses, and that no older client may displace, binds nothing, takes no
            // token and saves nothing. The limits are checked again after the bindings.
            let capacity = Capacity::of(state, source);
            if capacity.any() && !can_displace(state, device_id, source, &capacity) {
                return (Registration::Failed(capacity.refusal()), Vec::new());
            }
            // Only a new entry takes a token; a bound one (own or another key's) does not.
            if state.bindings.is_new_device(device_id) && !state.binding_rate.try_take(source) {
                return (
                    Registration::Failed(RegistrationError::BindingRateLimited),
                    Vec::new(),
                );
            }
            match state
                .bindings
                .bind_device(device_id, &key_text, unix_seconds())
            {
                BindOutcome::Bound | BindOutcome::Confirmed => {}
                BindOutcome::Conflict => {
                    return (
                        Registration::Failed(RegistrationError::DeviceBound),
                        Vec::new(),
                    );
                }
                BindOutcome::KeyFull => {
                    return (
                        Registration::Failed(RegistrationError::KeyDevicesFull),
                        Vec::new(),
                    );
                }
                BindOutcome::Full => {
                    return (
                        Registration::Failed(RegistrationError::BindingStateUnavailable),
                        Vec::new(),
                    );
                }
            }
            // The binding is acknowledged only once a copy holding it is on disk
            // (SEC-SERVER-BESITZER-60S): the copy is written outside the lock first.
            if state.binding_path.is_some() {
                let ack = *need.get_or_insert_with(|| state.bindings.ack_generation());
                if state.binding_saves.written() < ack {
                    return (Registration::Save, Vec::new());
                }
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
        }
        None => {
            if state.policy.require_key_login {
                return (
                    Registration::Failed(RegistrationError::KeyLoginRequired),
                    Vec::new(),
                );
            }
            if state.bindings.device_key(device_id).is_some() {
                return (
                    Registration::Failed(RegistrationError::DeviceBound),
                    Vec::new(),
                );
            }
        }
    }
    let capacity = Capacity::of(state, source);
    if capacity.any() {
        let victim = if identity.proven().is_some() {
            displacement(state, source, &capacity)
        } else {
            None
        };
        return match victim {
            Some(victim) => (Registration::Retry, vec![victim]),
            None => (Registration::Failed(capacity.refusal()), Vec::new()),
        };
    }
    let Some(id) = state.next_id.checked_add(1) else {
        return (
            Registration::Failed(RegistrationError::IdExhausted),
            Vec::new(),
        );
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

/// Whether a watcher may see a lookup's presence: a client without key login only an
/// unbound lookup, because it cannot show a proof (E60); proven ones with the owner's
/// access hash.
pub(super) fn watcher_admitted(state: &State, watcher_id: u64, lookup_id: &str) -> bool {
    let Some(client) = state.clients.get(&watcher_id) else {
        return false;
    };
    if client.identity.proven().is_none() {
        return state.bindings.lookup_key(lookup_id).is_none();
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

#[cfg(test)]
#[path = "bindings_limit_task_tests.rs"]
mod bindings_limit_task_tests;
