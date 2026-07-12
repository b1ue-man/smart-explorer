use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::{lock_state, send, Out, PeerPresence, State, Writer};

const MAX_WATCHES: usize = 256;
pub(super) const CAPABILITY: &str = "tracked_direct_v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DirectPeerIdentity {
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) node_id: String,
    pub(super) public_key: String,
    pub(super) fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectRequest {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) created_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectRequestReceipt {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) request_digest: String,
    pub(super) received_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectDecisionKind {
    Accepted,
    Rejected,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectDecision {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) request_digest: String,
    pub(super) decision: DirectDecisionKind,
    pub(super) decision_revision: u64,
    pub(super) decided_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SignedDirectDecisionReceipt {
    pub(super) request_id: String,
    pub(super) lookup_id: String,
    pub(super) requester: DirectPeerIdentity,
    pub(super) target: DirectPeerIdentity,
    pub(super) decision_digest: String,
    pub(super) decision: DirectDecisionKind,
    pub(super) decision_revision: u64,
    pub(super) received_at: i64,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) message: Option<String>,
    pub(super) hmac_proof: String,
    pub(super) identity_signature: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectRoute {
    Request,
    RequestReceipt,
    Decision,
    DecisionReceipt,
}

/// A `forwarded` ACK means only that this relay accepted the message and
/// enqueued it to at least one currently connected, compatible client writer.
/// It is not proof that the peer socket received or persisted the payload.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectRouteOutcome {
    Forwarded,
    TargetOffline,
}

pub(super) fn negotiate_capabilities(offered: Vec<String>) -> HashSet<String> {
    offered
        .into_iter()
        .filter(|capability| capability == CAPABILITY)
        .collect()
}

pub(super) fn capability_list(negotiated: &HashSet<String>) -> Vec<String> {
    if negotiated.contains(CAPABILITY) {
        vec![CAPABILITY.to_string()]
    } else {
        Vec::new()
    }
}

pub(super) fn route_request(
    origin_id: u64,
    origin: &Writer,
    request: SignedDirectRequest,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(origin_id, origin, state) {
        return;
    }
    let request_id = request.request_id.clone();
    let targets = capable_writer_for_lookup(&request.lookup_id, state)
        .map(|writer| vec![writer])
        .unwrap_or_default();
    let outcome = forward_to(targets, Out::DirectRequest { request });
    acknowledge(origin, request_id, DirectRoute::Request, outcome);
}

pub(super) fn route_request_receipt(
    origin_id: u64,
    origin: &Writer,
    receipt: SignedDirectRequestReceipt,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(origin_id, origin, state) {
        return;
    }
    let request_id = receipt.request_id.clone();
    let targets = capable_writers_by_device(&receipt.requester.device_id, state);
    let outcome = forward_to(targets, Out::DirectRequestReceipt { receipt });
    acknowledge(origin, request_id, DirectRoute::RequestReceipt, outcome);
}

pub(super) fn route_decision(
    origin_id: u64,
    origin: &Writer,
    decision: SignedDirectDecision,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(origin_id, origin, state) {
        return;
    }
    let request_id = decision.request_id.clone();
    let targets = capable_writers_by_device(&decision.requester.device_id, state);
    let outcome = forward_to(targets, Out::DirectDecision { decision });
    acknowledge(origin, request_id, DirectRoute::Decision, outcome);
}

pub(super) fn route_decision_receipt(
    origin_id: u64,
    origin: &Writer,
    receipt: SignedDirectDecisionReceipt,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(origin_id, origin, state) {
        return;
    }
    let request_id = receipt.request_id.clone();
    let targets = capable_writers_by_device(&receipt.target.device_id, state);
    let outcome = forward_to(targets, Out::DirectDecisionReceipt { receipt });
    acknowledge(origin, request_id, DirectRoute::DecisionReceipt, outcome);
}

pub(super) fn request_legacy(
    origin: &Writer,
    lookup_id: &str,
    presence: PeerPresence,
    state: &Arc<Mutex<State>>,
) {
    let target = writer_for_lookup(lookup_id, state);
    if let Some(target) = target {
        send(
            &target,
            &Out::DirectAccessRequest {
                lookup_id: lookup_id.to_string(),
                presence,
            },
        );
    } else {
        send(
            origin,
            &Out::Error {
                scope: "direct".into(),
                msg: "Direktgeraet nicht online".into(),
            },
        );
    }
}

pub(super) fn decision_legacy(
    lookup_id: &str,
    requester_device_id: &str,
    accepted: bool,
    presence: Option<PeerPresence>,
    msg: Option<String>,
    state: &Arc<Mutex<State>>,
) {
    for target in writers_by_device(requester_device_id, state) {
        send(
            &target,
            &Out::DirectAccessAccepted {
                lookup_id: lookup_id.to_string(),
                requester_device_id: requester_device_id.to_string(),
                accepted,
                presence: presence.clone(),
                msg: msg.clone(),
            },
        );
    }
}

pub(super) fn publish(id: u64, presence: PeerPresence, state: &Arc<Mutex<State>>) {
    let lookup_id = presence.relation_id.clone();
    let watchers = {
        let mut state = lock_state(state);
        state
            .direct
            .insert(lookup_id.clone(), (id, presence.clone()));
        if let Some(client) = state.clients.get_mut(&id) {
            client.direct_lookup_ids.insert(lookup_id.clone());
        }
        state.watchers.get(&lookup_id).cloned().unwrap_or_default()
    };
    notify_available(&lookup_id, &presence, watchers, state);
}

pub(super) fn unpublish(id: u64, lookup_id: &str, state: &Arc<Mutex<State>>) {
    let watchers = {
        let mut state = lock_state(state);
        if state.direct.get(lookup_id).map(|(owner, _)| *owner) == Some(id) {
            state.direct.remove(lookup_id);
        }
        if let Some(client) = state.clients.get_mut(&id) {
            client.direct_lookup_ids.remove(lookup_id);
        }
        state.watchers.get(lookup_id).cloned().unwrap_or_default()
    };
    notify_offline(lookup_id, watchers, state);
}

pub(super) fn watch(id: u64, origin: &Writer, lookup_id: &str, state: &Arc<Mutex<State>>) {
    let current = {
        let mut state = lock_state(state);
        if state
            .clients
            .get(&id)
            .map(|client| client.watched_lookup_ids.len() >= MAX_WATCHES)
            .unwrap_or(false)
        {
            send(
                origin,
                &Out::Error {
                    scope: "direct".into(),
                    msg: "too many watches".into(),
                },
            );
            return;
        }
        state
            .watchers
            .entry(lookup_id.to_string())
            .or_default()
            .insert(id);
        if let Some(client) = state.clients.get_mut(&id) {
            client.watched_lookup_ids.insert(lookup_id.to_string());
        }
        state.direct.get(lookup_id).map(|(_, p)| p.clone())
    };
    if let Some(presence) = current {
        send(
            origin,
            &Out::DirectAvailable {
                lookup_id: lookup_id.to_string(),
                presence,
            },
        );
    }
}

pub(super) fn unwatch(id: u64, lookup_id: &str, state: &Arc<Mutex<State>>) {
    let mut state = lock_state(state);
    if let Some(client) = state.clients.get_mut(&id) {
        client.watched_lookup_ids.remove(lookup_id);
    }
    if let Some(watchers) = state.watchers.get_mut(lookup_id) {
        watchers.remove(&id);
    }
}

fn require_capability(origin_id: u64, origin: &Writer, state: &Arc<Mutex<State>>) -> bool {
    let supported = lock_state(state)
        .clients
        .get(&origin_id)
        .is_some_and(|client| client.capabilities.contains(CAPABILITY));
    if !supported {
        send(
            origin,
            &Out::Error {
                scope: "tracked_direct".into(),
                msg: "tracked_direct_v1 capability was not negotiated".into(),
            },
        );
    }
    supported
}

fn forward_to(targets: Vec<Writer>, message: Out) -> DirectRouteOutcome {
    let mut forwarded = false;
    for target in targets {
        forwarded |= target.send(message.clone()).is_ok();
    }
    if forwarded {
        DirectRouteOutcome::Forwarded
    } else {
        DirectRouteOutcome::TargetOffline
    }
}

fn acknowledge(
    origin: &Writer,
    request_id: String,
    route: DirectRoute,
    outcome: DirectRouteOutcome,
) {
    send(
        origin,
        &Out::DirectRouteAck {
            request_id,
            route,
            outcome,
        },
    );
}

fn capable_writer_for_lookup(lookup_id: &str, state: &Arc<Mutex<State>>) -> Option<Writer> {
    let state = lock_state(state);
    state
        .direct
        .get(lookup_id)
        .and_then(|(owner_id, _)| state.clients.get(owner_id))
        .filter(|client| client.capabilities.contains(CAPABILITY))
        .map(|client| client.writer.clone())
}

fn capable_writers_by_device(device_id: &str, state: &Arc<Mutex<State>>) -> Vec<Writer> {
    let state = lock_state(state);
    state
        .clients
        .values()
        .filter(|client| client.device_id == device_id && client.capabilities.contains(CAPABILITY))
        .map(|client| client.writer.clone())
        .collect()
}

fn writer_for_lookup(lookup_id: &str, state: &Arc<Mutex<State>>) -> Option<Writer> {
    let state = lock_state(state);
    state
        .direct
        .get(lookup_id)
        .and_then(|(owner_id, _)| state.clients.get(owner_id))
        .map(|client| client.writer.clone())
}

fn writers_by_device(device_id: &str, state: &Arc<Mutex<State>>) -> Vec<Writer> {
    let state = lock_state(state);
    state
        .clients
        .values()
        .filter(|client| client.device_id == device_id)
        .map(|client| client.writer.clone())
        .collect()
}

fn notify_available(
    lookup_id: &str,
    presence: &PeerPresence,
    watchers: HashSet<u64>,
    state: &Arc<Mutex<State>>,
) {
    for writer in writers_for(watchers, state) {
        send(
            &writer,
            &Out::DirectAvailable {
                lookup_id: lookup_id.to_string(),
                presence: presence.clone(),
            },
        );
    }
}

fn notify_offline(lookup_id: &str, watchers: HashSet<u64>, state: &Arc<Mutex<State>>) {
    for writer in writers_for(watchers, state) {
        send(
            &writer,
            &Out::DirectOffline {
                lookup_id: lookup_id.to_string(),
            },
        );
    }
}

fn writers_for(ids: HashSet<u64>, state: &Arc<Mutex<State>>) -> Vec<Writer> {
    let state = lock_state(state);
    ids.into_iter()
        .filter_map(|id| state.clients.get(&id).map(|client| client.writer.clone()))
        .collect()
}
