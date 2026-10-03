//! Ownership and access proofs of Direct publications/subscriptions (FC4).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::bindings::BindOutcome;
use crate::limits::{validate_identifier, validate_presence, RetainError,
    MAX_PUBLISHED_DIRECTS_PER_CLIENT, MAX_WATCHES_PER_CLIENT};
use crate::state::{lock_state, watcher_admitted, State};
use crate::{send, Out, PeerPresence, Writer};

pub(super) fn origin_matches(state: &State, id: u64, device: &str, public_key: &str, node: &str) -> bool {
    state.clients.get(&id).is_some_and(|client| {
        client.device_id == device && client.identity.proven().is_none_or(|key| {
            crate::login::parse_key(public_key).as_ref() == Some(key)
                && crate::login::parse_key(node).as_ref() == Some(key)
        })
    })
}

pub(super) fn require_origin(
    state: &Arc<Mutex<State>>, id: u64, writer: &Writer, presence: &PeerPresence,
) -> bool {
    let allowed = origin_matches(&lock_state(state), id, &presence.device_id,
        &presence.public_key, &presence.node_id);
    if !allowed { reject(writer, RetainError::Denied("registered identity")); }
    allowed
}

pub(super) fn require_identity(
    state: &Arc<Mutex<State>>, id: u64, writer: &Writer, peer: &crate::direct_messages::DirectPeerIdentity,
) -> bool {
    let allowed = origin_matches(&lock_state(state), id, &peer.device_id, &peer.public_key, &peer.node_id);
    if !allowed { reject(writer, RetainError::Denied("registered identity")); }
    allowed
}

pub(super) fn require_owner(state: &Arc<Mutex<State>>, id: u64, writer: &Writer, lookup: &str) -> bool {
    let state = lock_state(state);
    let key = state.clients.get(&id).and_then(|client| client.identity.proven()).map(ToString::to_string);
    let allowed = match state.bindings.lookup_key(lookup) {
        Some(owner) => key.as_deref() == Some(owner),
        None => state.direct.get(lookup).is_some_and(|(owner, _)| *owner == id),
    };
    if !allowed { reject(writer, RetainError::Denied("lookup ownership")); }
    allowed
}

pub(super) fn publish(
    id: u64, writer: &Writer, presence: PeerPresence, access_hash: Option<String>, state: &Arc<Mutex<State>>,
) {
    let result = validate_presence(&presence).and_then(|_| {
        if presence.kind != "direct" { return Err(RetainError::InvalidField("presence kind")); }
        let mut state = lock_state(state);
        if !origin_matches(&state, id, &presence.device_id, &presence.public_key, &presence.node_id) {
            return Err(RetainError::Denied("registered identity"));
        }
        let client = &state.clients[&id];
        let lookup = &presence.relation_id;
        if !client.direct_lookup_ids.contains(lookup) && client.direct_lookup_ids.len() >= MAX_PUBLISHED_DIRECTS_PER_CLIENT {
            return Err(RetainError::Limit("published directs"));
        }
        let key = client.identity.proven().map(ToString::to_string);
        if let Some(bound) = state.bindings.lookup_key(lookup) {
            if key.as_deref() != Some(bound) { return Err(RetainError::Bound("lookup")); }
        }
        if let Some(key) = key {
            let hash = access_hash.filter(|hash| crate::access::parse_hash(hash).is_some())
                .ok_or(RetainError::Denied("relation access hash"))?;
            match state.bindings.bind_lookup(lookup, &key, Some(hash), crate::discovery_state::unix_seconds()) {
                BindOutcome::Conflict => return Err(RetainError::Bound("lookup")),
                BindOutcome::Full => return Err(RetainError::Limit("key bindings")),
                _ => {}
            }
            if state.persist_bindings().is_err() { return Err(RetainError::Denied("persistent key state")); }
        }
        if let Some((previous, _)) = state.direct.insert(lookup.clone(), (id, presence.clone())) {
            if previous != id {
                if let Some(client) = state.clients.get_mut(&previous) { client.direct_lookup_ids.remove(lookup); }
            }
        }
        if let Some(client) = state.clients.get_mut(&id) { client.direct_lookup_ids.insert(lookup.clone()); }
        Ok(state.watchers.get(lookup).cloned().unwrap_or_default())
    });
    match result {
        Ok(watchers) => {
            let message = Out::DirectAvailable { lookup_id: presence.relation_id.clone(), presence: presence.clone() };
            for target in writers_for(state, watchers, &presence.relation_id) { target.offer(&message); }
        }
        Err(error) => reject(writer, error),
    }
}

pub(super) fn watch(
    id: u64, writer: &Writer, lookup: &str, access_proof: Option<String>, state: &Arc<Mutex<State>>,
) {
    let result = validate_identifier("lookup id", lookup).and_then(|_| {
        let mut state = lock_state(state);
        let Some(client) = state.clients.get(&id) else { return Err(RetainError::Denied("registration")); };
        if !client.watched_lookup_ids.contains(lookup) && client.watched_lookup_ids.len() >= MAX_WATCHES_PER_CLIENT {
            return Err(RetainError::Limit("watches"));
        }
        let proven = client.identity.proven().is_some();
        if proven {
            let hash = access_proof.as_deref().and_then(crate::access::proof_hash)
                .ok_or(RetainError::Denied("relation access proof"))?;
            if state.bindings.lookup_access(lookup).is_some_and(|expected| expected != hash) {
                return Err(RetainError::Denied("relation access proof"));
            }
            state.watch_access.insert((id, lookup.to_string()), hash);
        }
        state.watchers.entry(lookup.to_string()).or_default().insert(id);
        if let Some(client) = state.clients.get_mut(&id) { client.watched_lookup_ids.insert(lookup.to_string()); }
        Ok(watcher_admitted(&state, id, lookup).then(|| state.direct.get(lookup)
            .map(|(_, presence)| presence.clone())).flatten())
    });
    match result {
        Ok(Some(presence)) => { send(writer, &Out::DirectAvailable { lookup_id: lookup.into(), presence }); }
        Ok(None) => {}
        Err(error) => reject(writer, error),
    }
}

pub(super) fn unpublish(id: u64, lookup: &str, state: &Arc<Mutex<State>>) {
    let watchers = {
        let mut state = lock_state(state);
        if state.direct.get(lookup).map(|(owner, _)| *owner) != Some(id) { return; }
        state.direct.remove(lookup);
        if let Some(client) = state.clients.get_mut(&id) { client.direct_lookup_ids.remove(lookup); }
        state.watchers.get(lookup).cloned().unwrap_or_default()
    };
    for target in writers_for(state, watchers, lookup) { send(&target, &Out::DirectOffline { lookup_id: lookup.into() }); }
}

pub(super) fn unwatch(id: u64, lookup: &str, state: &Arc<Mutex<State>>) {
    let mut state = lock_state(state);
    state.watch_access.remove(&(id, lookup.to_string()));
    if let Some(client) = state.clients.get_mut(&id) { client.watched_lookup_ids.remove(lookup); }
    if let Some(watchers) = state.watchers.get_mut(lookup) {
        watchers.remove(&id);
        if watchers.is_empty() { state.watchers.remove(lookup); }
    }
}

fn writers_for(state: &Arc<Mutex<State>>, ids: HashSet<u64>, lookup: &str) -> Vec<Writer> {
    let state = lock_state(state);
    ids.into_iter().filter(|id| watcher_admitted(&state, *id, lookup))
        .filter_map(|id| state.clients.get(&id).map(|client| client.writer.clone())).collect()
}

pub(super) fn reject(writer: &Writer, error: RetainError) {
    send(writer, &Out::Error { scope: "direct".into(), msg: error.message() });
}
