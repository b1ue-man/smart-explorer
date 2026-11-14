//! Ownership and access proofs of Direct publications/subscriptions (FC4).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::bindings::{BindOutcome, PendingCopy};
use crate::limits::{
    validate_identifier, validate_presence, RetainError, MAX_PUBLISHED_DIRECTS_PER_CLIENT,
    MAX_WATCHES_PER_CLIENT,
};
use crate::state::{lock_state, watcher_admitted, RegistrationError, State};
use crate::{send, Out, PeerPresence, Writer};

pub(super) fn origin_matches(
    state: &State,
    id: u64,
    device: &str,
    public_key: &str,
    node: &str,
) -> bool {
    state.clients.get(&id).is_some_and(|client| {
        client.device_id == device
            && client.identity.proven().is_none_or(|key| {
                crate::login::parse_key(public_key).as_ref() == Some(key)
                    && crate::login::parse_key(node).as_ref() == Some(key)
            })
    })
}

pub(super) fn require_origin(
    state: &Arc<Mutex<State>>,
    id: u64,
    writer: &Writer,
    presence: &PeerPresence,
) -> bool {
    let allowed = origin_matches(
        &lock_state(state),
        id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
    );
    if !allowed {
        reject(writer, RetainError::Denied("registered identity"));
    }
    allowed
}

pub(super) fn require_identity(
    state: &Arc<Mutex<State>>,
    id: u64,
    writer: &Writer,
    peer: &crate::direct_messages::DirectPeerIdentity,
) -> bool {
    let allowed = origin_matches(
        &lock_state(state),
        id,
        &peer.device_id,
        &peer.public_key,
        &peer.node_id,
    );
    if !allowed {
        reject(writer, RetainError::Denied("registered identity"));
    }
    allowed
}

pub(super) fn require_owner(
    state: &Arc<Mutex<State>>,
    id: u64,
    writer: &Writer,
    lookup: &str,
) -> bool {
    let state = lock_state(state);
    let key = state
        .clients
        .get(&id)
        .and_then(|client| client.identity.proven())
        .map(ToString::to_string);
    let allowed = match state.bindings.lookup_key(lookup) {
        Some(owner) => key.as_deref() == Some(owner),
        None => state
            .direct
            .get(lookup)
            .is_some_and(|(owner, _)| *owner == id),
    };
    if !allowed {
        reject(writer, RetainError::Denied("lookup ownership"));
    }
    allowed
}

/// Whether `id` is a registered client without key login (E60). An unknown id is not
/// one: its publication then fails the registration check as before.
fn is_legacy_client(id: u64, state: &Arc<Mutex<State>>) -> bool {
    lock_state(state)
        .clients
        .get(&id)
        .is_some_and(|client| client.identity.proven().is_none())
}

pub(super) fn publish(
    id: u64,
    writer: &Writer,
    presence: PeerPresence,
    access_hash: Option<String>,
    state: &Arc<Mutex<State>>,
) {
    // E60: a client without key login publishes nothing, not even for an unbound lookup.
    // The check comes first, so the refusal does not depend on the content of the presence.
    if is_legacy_client(id, state) {
        reject(writer, Refusal::KeyLogin);
        return;
    }
    let result = validate_presence(&presence).and_then(|_| {
        if presence.kind != "direct" {
            return Err(RetainError::InvalidField("presence kind"));
        }
        let copy = publish_bind(id, &presence, access_hash, state)?;
        // The copy holding the binding reaches the disk outside the state lock (R1).
        if let Some(copy) = copy {
            copy.write(state)
                .map_err(|_| RetainError::Denied("persistent key state"))?;
        }
        publish_admit(id, &presence, state)
    });
    match result {
        Ok(watchers) => {
            let message = Out::DirectAvailable {
                lookup_id: presence.relation_id.clone(),
                presence: presence.clone(),
            };
            for target in writers_for(state, watchers, &presence.relation_id) {
                target.offer(&message);
            }
        }
        Err(error) => reject(writer, error),
    }
}

/// First part of a publication, under the state lock: the checks, the binding of
/// the lookup to a proven key, and the copy of the bindings that must reach the disk
/// before the entry is visible. The copy is `None` without a state file.
fn publish_bind(
    id: u64,
    presence: &PeerPresence,
    access_hash: Option<String>,
    state: &Arc<Mutex<State>>,
) -> Result<Option<PendingCopy>, RetainError> {
    let mut state = lock_state(state);
    if !origin_matches(
        &state,
        id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
    ) {
        return Err(RetainError::Denied("registered identity"));
    }
    let client = &state.clients[&id];
    let lookup = &presence.relation_id;
    if !client.direct_lookup_ids.contains(lookup)
        && client.direct_lookup_ids.len() >= MAX_PUBLISHED_DIRECTS_PER_CLIENT
    {
        return Err(RetainError::Limit("published directs"));
    }
    let key = client.identity.proven().map(ToString::to_string);
    let source = client.source;
    if let Some(bound) = state.bindings.lookup_key(lookup) {
        if key.as_deref() != Some(bound) {
            return Err(RetainError::Bound("lookup"));
        }
    }
    // Only a proven client binds: `publish` refused every other client before (E60).
    let Some(key) = key else {
        return Err(RetainError::Denied("registered identity"));
    };
    let hash = access_hash
        .filter(|hash| crate::access::parse_hash(hash).is_some())
        .ok_or(RetainError::Denied("relation access hash"))?;
    // Only a new lookup binding takes a token of the source (R-SERVER-02).
    if state.bindings.is_new_lookup(lookup) && !state.binding_rate.try_take(source) {
        return Err(RetainError::Limit(
            "new key bindings from this network; retry later",
        ));
    }
    match state.bindings.bind_lookup(
        lookup,
        &key,
        Some(hash),
        crate::discovery_state::unix_seconds(),
    ) {
        BindOutcome::Conflict => return Err(RetainError::Bound("lookup")),
        BindOutcome::Full => return Err(RetainError::Limit("key bindings")),
        BindOutcome::KeyFull => return Err(RetainError::Limit("key bindings for this device key")),
        BindOutcome::Bound | BindOutcome::Confirmed => {}
    }
    Ok(state.take_publication_copy())
}

/// Second part of a publication, under the state lock again after the copy was
/// written: the identity, the publication limit and the binding of the lookup must
/// still hold. Then the entry is recorded. Returns the watchers to notify.
fn publish_admit(
    id: u64,
    presence: &PeerPresence,
    state: &Arc<Mutex<State>>,
) -> Result<HashSet<u64>, RetainError> {
    let mut state = lock_state(state);
    if !origin_matches(
        &state,
        id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
    ) {
        return Err(RetainError::Denied("registered identity"));
    }
    let client = &state.clients[&id];
    let lookup = &presence.relation_id;
    if !client.direct_lookup_ids.contains(lookup)
        && client.direct_lookup_ids.len() >= MAX_PUBLISHED_DIRECTS_PER_CLIENT
    {
        return Err(RetainError::Limit("published directs"));
    }
    // E60: only the entry of a proven client becomes visible.
    let Some(key) = client.identity.proven().map(ToString::to_string) else {
        return Err(RetainError::Denied("registered identity"));
    };
    if let Some(bound) = state.bindings.lookup_key(lookup) {
        if key.as_str() != bound {
            return Err(RetainError::Bound("lookup"));
        }
    }
    if let Some((previous, _)) = state.direct.insert(lookup.clone(), (id, presence.clone())) {
        if previous != id {
            if let Some(client) = state.clients.get_mut(&previous) {
                client.direct_lookup_ids.remove(lookup);
            }
        }
    }
    if let Some(client) = state.clients.get_mut(&id) {
        client.direct_lookup_ids.insert(lookup.clone());
    }
    Ok(state.watchers.get(lookup).cloned().unwrap_or_default())
}

pub(super) fn watch(
    id: u64,
    writer: &Writer,
    lookup: &str,
    access_proof: Option<String>,
    state: &Arc<Mutex<State>>,
) {
    let result = validate_identifier("lookup id", lookup)
        .map_err(Refusal::from)
        .and_then(|_| {
            let mut state = lock_state(state);
            let Some(client) = state.clients.get(&id) else {
                return Err(RetainError::Denied("registration").into());
            };
            if !client.watched_lookup_ids.contains(lookup)
                && client.watched_lookup_ids.len() >= MAX_WATCHES_PER_CLIENT
            {
                return Err(RetainError::Limit("watches").into());
            }
            let proven = client.identity.proven().is_some();
            // E60: without key login a watcher sees unbound lookups only. This check comes
            // before the registration below, so a refused watcher is not kept.
            if !proven && state.bindings.lookup_key(lookup).is_some() {
                return Err(Refusal::KeyLogin);
            }
            if proven {
                let hash = access_proof
                    .as_deref()
                    .and_then(crate::access::proof_hash)
                    .ok_or(RetainError::Denied("relation access proof"))?;
                if state
                    .bindings
                    .lookup_access(lookup)
                    .is_some_and(|expected| expected != hash)
                {
                    return Err(RetainError::Denied("relation access proof").into());
                }
                state.watch_access.insert((id, lookup.to_string()), hash);
            }
            state
                .watchers
                .entry(lookup.to_string())
                .or_default()
                .insert(id);
            if let Some(client) = state.clients.get_mut(&id) {
                client.watched_lookup_ids.insert(lookup.to_string());
            }
            Ok(watcher_admitted(&state, id, lookup)
                .then(|| {
                    state
                        .direct
                        .get(lookup)
                        .map(|(_, presence)| presence.clone())
                })
                .flatten())
        });
    match result {
        Ok(Some(presence)) => {
            send(
                writer,
                &Out::DirectAvailable {
                    lookup_id: lookup.into(),
                    presence,
                },
            );
        }
        Ok(None) => {}
        Err(error) => reject(writer, error),
    }
}

pub(super) fn unpublish(id: u64, lookup: &str, state: &Arc<Mutex<State>>) {
    let watchers = {
        let mut state = lock_state(state);
        if state.direct.get(lookup).map(|(owner, _)| *owner) != Some(id) {
            return;
        }
        state.direct.remove(lookup);
        if let Some(client) = state.clients.get_mut(&id) {
            client.direct_lookup_ids.remove(lookup);
        }
        state.watchers.get(lookup).cloned().unwrap_or_default()
    };
    for target in writers_for(state, watchers, lookup) {
        send(
            &target,
            &Out::DirectOffline {
                lookup_id: lookup.into(),
            },
        );
    }
}

pub(super) fn unwatch(id: u64, lookup: &str, state: &Arc<Mutex<State>>) {
    let mut state = lock_state(state);
    state.watch_access.remove(&(id, lookup.to_string()));
    if let Some(client) = state.clients.get_mut(&id) {
        client.watched_lookup_ids.remove(lookup);
    }
    if let Some(watchers) = state.watchers.get_mut(lookup) {
        watchers.remove(&id);
        if watchers.is_empty() {
            state.watchers.remove(lookup);
        }
    }
}

fn writers_for(state: &Arc<Mutex<State>>, ids: HashSet<u64>, lookup: &str) -> Vec<Writer> {
    let state = lock_state(state);
    ids.into_iter()
        .filter(|id| watcher_admitted(&state, *id, lookup))
        .filter_map(|id| state.clients.get(&id).map(|client| client.writer.clone()))
        .collect()
}

/// E60, S15: a client without key login reaches no bound lookup on any request path
/// (a legacy request, a legacy decision, a signed request with or without the legacy
/// bridge, a signed decision receipt); it gets the text of a missing key login, as
/// for a publication. `true` when the request may go on; an unbound lookup stays
/// reachable (mixed operation). The two signed paths of a lookup's owner (request
/// receipt, decision) need `require_owner`, and only a proven client owns a lookup.
pub(super) fn admit_legacy_route(
    id: u64,
    writer: &Writer,
    lookup_id: &str,
    state: &Arc<Mutex<State>>,
) -> bool {
    let refused = {
        let state = lock_state(state);
        state
            .clients
            .get(&id)
            .is_some_and(|client| client.identity.proven().is_none())
            && state.bindings.lookup_key(lookup_id).is_some()
    };
    if refused {
        reject(writer, Refusal::KeyLogin);
    }
    !refused
}

/// A refusal of a Direct publication or watch: the text of the list, or the existing
/// text of a missing key login (E60).
enum Refusal {
    Entry(RetainError),
    KeyLogin,
}

impl From<RetainError> for Refusal {
    fn from(error: RetainError) -> Self {
        Self::Entry(error)
    }
}

fn reject(writer: &Writer, refusal: impl Into<Refusal>) {
    let msg = match refusal.into() {
        Refusal::Entry(error) => error.message(),
        Refusal::KeyLogin => RegistrationError::KeyLoginRequired.message().into(),
    };
    send(
        writer,
        &Out::Error {
            scope: "direct".into(),
            msg,
        },
    );
}

#[cfg(test)]
#[path = "unbelegt_legacy_task_tests.rs"]
mod unbelegt_legacy_task_tests;
