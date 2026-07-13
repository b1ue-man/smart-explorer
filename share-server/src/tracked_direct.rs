use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::direct_validation;
use super::limits::{
    validate_identifier, validate_presence, RetainError, MAX_PUBLISHED_DIRECTS_PER_CLIENT,
    MAX_WATCHES_PER_CLIENT,
};
use super::state::{lock_state, State};
use super::{send, Out, PeerPresence, Writer};

#[cfg(test)]
pub(super) use super::direct_messages::DirectDecisionKind;
pub(super) use super::direct_messages::{
    DirectPeerIdentity, DirectRoute, DirectRouteOutcome, SignedDirectDecision,
    SignedDirectDecisionReceipt, SignedDirectRequest, SignedDirectRequestReceipt,
};

pub(super) const CAPABILITY: &str = "tracked_direct_v1";

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
    legacy_presence: Option<PeerPresence>,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(origin_id, origin, state) {
        return;
    }
    if let Err(error) = direct_validation::validate_request(&request) {
        send_retain_error(origin, error);
        return;
    }
    if let Some(presence) = &legacy_presence {
        if let Err(error) = direct_validation::validate_legacy_bridge(&request, presence) {
            send_retain_error(origin, error);
            return;
        }
    }
    let request_id = request.request_id.clone();
    let outcome = match request_target(&request.lookup_id, state) {
        Some((target, true)) => forward_to(vec![target], Out::DirectRequest { request }),
        Some((target, false)) => match legacy_presence {
            Some(presence) => {
                if send(
                    &target,
                    &Out::DirectAccessRequest {
                        lookup_id: request.lookup_id,
                        presence,
                    },
                ) {
                    DirectRouteOutcome::LegacyForwarded
                } else {
                    DirectRouteOutcome::TargetOffline
                }
            }
            _ => DirectRouteOutcome::TargetOffline,
        },
        None => DirectRouteOutcome::TargetOffline,
    };
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
    if let Err(error) = direct_validation::validate_request_receipt(&receipt) {
        send_retain_error(origin, error);
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
    if let Err(error) = direct_validation::validate_decision(&decision) {
        send_retain_error(origin, error);
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
    if let Err(error) = direct_validation::validate_decision_receipt(&receipt) {
        send_retain_error(origin, error);
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
    if let Err(error) = direct_validation::validate_legacy_request(lookup_id, &presence) {
        send_retain_error(origin, error);
        return;
    }
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
    origin: &Writer,
    lookup_id: &str,
    requester_device_id: &str,
    accepted: bool,
    presence: Option<PeerPresence>,
    msg: Option<String>,
    state: &Arc<Mutex<State>>,
) {
    if let Err(error) = direct_validation::validate_legacy_decision(
        lookup_id,
        requester_device_id,
        presence.as_ref(),
        msg.as_deref(),
    ) {
        send_retain_error(origin, error);
        return;
    }
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

pub(super) fn publish(id: u64, origin: &Writer, presence: PeerPresence, state: &Arc<Mutex<State>>) {
    if let Err(error) = validate_presence(&presence) {
        send_retain_error(origin, error);
        return;
    }
    let lookup_id = presence.relation_id.clone();
    let retain_result = {
        let mut state = lock_state(state);
        let Some(client) = state.clients.get(&id) else {
            return;
        };
        if client.device_id != presence.device_id {
            Err(RetainError::InvalidField("presence device id"))
        } else if !client.direct_lookup_ids.contains(&lookup_id)
            && client.direct_lookup_ids.len() >= MAX_PUBLISHED_DIRECTS_PER_CLIENT
        {
            Err(RetainError::Limit("published directs"))
        } else {
            state
                .direct
                .insert(lookup_id.clone(), (id, presence.clone()));
            if let Some(client) = state.clients.get_mut(&id) {
                client.direct_lookup_ids.insert(lookup_id.clone());
            }
            Ok(state.watchers.get(&lookup_id).cloned().unwrap_or_default())
        }
    };
    let watchers = match retain_result {
        Ok(watchers) => watchers,
        Err(error) => {
            send_retain_error(origin, error);
            return;
        }
    };
    notify_available(&lookup_id, &presence, watchers, state);
}

pub(super) fn unpublish(id: u64, lookup_id: &str, state: &Arc<Mutex<State>>) {
    let watchers = {
        let mut state = lock_state(state);
        let removed = state.direct.get(lookup_id).map(|(owner, _)| *owner) == Some(id);
        if removed {
            state.direct.remove(lookup_id);
        }
        if let Some(client) = state.clients.get_mut(&id) {
            client.direct_lookup_ids.remove(lookup_id);
        }
        removed.then(|| state.watchers.get(lookup_id).cloned().unwrap_or_default())
    };
    if let Some(watchers) = watchers {
        notify_offline(lookup_id, watchers, state);
    }
}

pub(super) fn watch(id: u64, origin: &Writer, lookup_id: &str, state: &Arc<Mutex<State>>) {
    if let Err(error) = validate_identifier("lookup id", lookup_id) {
        send_retain_error(origin, error);
        return;
    }
    let retain_result = {
        let mut state = lock_state(state);
        let Some(client) = state.clients.get(&id) else {
            return;
        };
        if !client.watched_lookup_ids.contains(lookup_id)
            && client.watched_lookup_ids.len() >= MAX_WATCHES_PER_CLIENT
        {
            Err(RetainError::Limit("watches"))
        } else {
            state
                .watchers
                .entry(lookup_id.to_string())
                .or_default()
                .insert(id);
            if let Some(client) = state.clients.get_mut(&id) {
                client.watched_lookup_ids.insert(lookup_id.to_string());
            }
            Ok(state
                .direct
                .get(lookup_id)
                .map(|(_, presence)| presence.clone()))
        }
    };
    let current = match retain_result {
        Ok(current) => current,
        Err(error) => {
            send_retain_error(origin, error);
            return;
        }
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
    let remove_lookup = if let Some(watchers) = state.watchers.get_mut(lookup_id) {
        watchers.remove(&id);
        watchers.is_empty()
    } else {
        false
    };
    if remove_lookup {
        state.watchers.remove(lookup_id);
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
        forwarded |= send(&target, &message);
    }
    if forwarded {
        DirectRouteOutcome::Forwarded
    } else {
        DirectRouteOutcome::TargetOffline
    }
}

fn send_retain_error(origin: &Writer, error: RetainError) {
    send(
        origin,
        &Out::Error {
            scope: "direct".into(),
            msg: error.message(),
        },
    );
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

fn request_target(lookup_id: &str, state: &Arc<Mutex<State>>) -> Option<(Writer, bool)> {
    let state = lock_state(state);
    state
        .direct
        .get(lookup_id)
        .and_then(|(owner_id, _)| state.clients.get(owner_id))
        .map(|client| {
            (
                client.writer.clone(),
                client.capabilities.contains(CAPABILITY),
            )
        })
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
