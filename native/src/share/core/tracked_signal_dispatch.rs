use std::sync::{Arc, Mutex};

use super::core::now_secs;
use super::configuration_runtime::RuntimeConfiguration;
use super::discovery_signal_commands::DiscoverySignalRuntime;
use super::discovery_signal_dispatch::DiscoveryDispatchOutcome;
use super::direct_ledger::{
    DirectEnvelopeKind, DirectRelayOutcome, DirectRequestDirection, DirectRequestEntry,
};
use super::direct_protocol::{DirectPeerIdentity, DirectRequestId, SignedDirectRequest};
use super::direct_signal_event::DirectSignalEvent;
use super::profiles::ShareProfiles;
use super::signal_auth::handle_server_msg;
use super::signal_connection::SignalConnection;
use super::tracked_signal_verify::{
    verify_decision_for_requester, verify_decision_receipt_for_target, verify_request_for_target,
    verify_request_receipt_for_requester,
};
use super::types::{DirectContact, ShareAuthState, ShareEvent};
use super::wire::{DirectRoute, DirectRouteOutcome, TrackedDirectServerMsg};

pub(super) fn parse_tracked_server_message(
    line: &str,
) -> Result<Option<TrackedDirectServerMsg>, String> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    let Some(kind) = value.get("t").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    if !matches!(
        kind,
        "direct_request"
            | "direct_request_receipt"
            | "direct_decision"
            | "direct_decision_receipt"
            | "direct_route_ack"
    ) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(super) enum SignalDispatchOutcome {
    Continue,
    Pong,
    Reconnect,
}

pub(super) fn dispatch_server_line(
    line: &str,
    tracked_direct: bool,
    discovery_exchange: bool,
    discovery: &mut DiscoverySignalRuntime,
    signal: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    events: &crossbeam_channel::Sender<ShareEvent>,
    configuration: &mut RuntimeConfiguration<'_>,
) -> SignalDispatchOutcome {
    match discovery.dispatch_server_line(
        line,
        signal,
        discovery_exchange,
        events,
        configuration,
        tracked_direct,
    ) {
        DiscoveryDispatchOutcome::Handled => return SignalDispatchOutcome::Continue,
        DiscoveryDispatchOutcome::Reconnect => return SignalDispatchOutcome::Reconnect,
        DiscoveryDispatchOutcome::NotDiscovery => {}
    }
    match parse_tracked_server_message(line) {
        Ok(Some(message)) if tracked_direct => {
            handle_tracked_server_message(message, auth, events);
            SignalDispatchOutcome::Continue
        }
        Ok(Some(_)) => {
            let _ = events.send(ShareEvent::Error(
                "Tracked-Direct-Nachricht ohne ausgehandelte Faehigkeit verworfen".into(),
            ));
            SignalDispatchOutcome::Continue
        }
        Ok(None) => {
            if is_discovery_error(line) {
                discovery.handle_server_discovery_error(events);
            }
            if handle_server_msg(line, auth, events) {
                SignalDispatchOutcome::Pong
            } else {
                SignalDispatchOutcome::Continue
            }
        }
        Err(error) => {
            let _ = events.send(ShareEvent::Error(format!("Server-Nachricht: {error}")));
            SignalDispatchOutcome::Continue
        }
    }
}

fn is_discovery_error(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .map_or(false, |value| {
            value.get("t").and_then(serde_json::Value::as_str) == Some("error")
                && value.get("scope").and_then(serde_json::Value::as_str) == Some("discovery")
        })
}

pub(super) fn handle_tracked_server_message(
    message: TrackedDirectServerMsg,
    auth: &Arc<Mutex<ShareAuthState>>,
    events: &crossbeam_channel::Sender<ShareEvent>,
) {
    match verified_event(message, auth, now_secs()) {
        Ok(event) => {
            let _ = events.send(ShareEvent::DirectSignal(event));
        }
        Err(error) => {
            let _ = events.send(ShareEvent::Error(format!(
                "Tracked-Direct-Nachricht verworfen: {error}"
            )));
        }
    }
}

fn verified_event(
    message: TrackedDirectServerMsg,
    auth: &Arc<Mutex<ShareAuthState>>,
    now: i64,
) -> Result<DirectSignalEvent, String> {
    match message {
        TrackedDirectServerMsg::Request { request } => {
            let (lookup_id, local, secret) = {
                let state = auth.lock().map_err(|_| "Share-State gesperrt")?;
                (
                    state.identity.direct_lookup_id.clone(),
                    local_identity(&state.identity),
                    state.direct_secret.clone(),
                )
            };
            verify_request_for_target(&request, &lookup_id, &local, &secret, now)
                .map_err(|error| error.to_string())?;
            Ok(DirectSignalEvent::RequestReceived {
                request,
                received_at: now,
            })
        }
        TrackedDirectServerMsg::RequestReceipt { receipt } => {
            let (request, contact, local) = outgoing_context(auth, &receipt.request_id)?;
            let secret = contact_secret(&contact)?;
            verify_request_receipt_for_requester(&receipt, &request, &local, &secret, now)
                .map_err(|error| error.to_string())?;
            Ok(DirectSignalEvent::RequestReceiptReceived {
                receipt,
                received_at: now,
            })
        }
        TrackedDirectServerMsg::Decision { decision } => {
            let (request, contact, local) = outgoing_context(auth, &decision.request_id)?;
            let secret = contact_secret(&contact)?;
            verify_decision_for_requester(&decision, &request, &local, &secret, now)
                .map_err(|error| error.to_string())?;
            Ok(DirectSignalEvent::DecisionReceived {
                decision,
                received_at: now,
            })
        }
        TrackedDirectServerMsg::DecisionReceipt { receipt } => {
            let (decision, local, secret) = {
                let state = auth.lock().map_err(|_| "Share-State gesperrt")?;
                let entry = find_entry(&state.direct_requests, &receipt.request_id)?;
                if entry.direction != DirectRequestDirection::Incoming {
                    return Err("Entscheidungsbestaetigung hat falsche Richtung".into());
                }
                let decision = entry
                    .decision
                    .clone()
                    .ok_or("Entscheidung fuer Bestaetigung fehlt")?;
                (
                    decision,
                    local_identity(&state.identity),
                    state.direct_secret.clone(),
                )
            };
            verify_decision_receipt_for_target(&receipt, &decision, &local, &secret, now)
                .map_err(|error| error.to_string())?;
            Ok(DirectSignalEvent::DecisionReceiptReceived {
                receipt,
                received_at: now,
            })
        }
        TrackedDirectServerMsg::RouteAck {
            request_id,
            route,
            outcome,
        } => relay_event(auth, request_id, route, outcome, now),
    }
}

fn outgoing_context(
    auth: &Arc<Mutex<ShareAuthState>>,
    request_id: &DirectRequestId,
) -> Result<(SignedDirectRequest, DirectContact, DirectPeerIdentity), String> {
    let state = auth.lock().map_err(|_| "Share-State gesperrt")?;
    let (request, contact_id) = match find_entry(&state.direct_requests, request_id) {
        Ok(entry) if entry.direction == DirectRequestDirection::Outgoing => (
            entry.record.request.clone(),
            entry.contact_id.clone().ok_or("Kontaktbezug fehlt")?,
        ),
        Ok(_) => return Err("Direktnachricht hat falsche Richtung".into()),
        Err(_) => {
            let tombstone = state
                .direct_request_tombstones
                .iter()
                .find(|tombstone| {
                    tombstone.request.request_id == *request_id
                        && tombstone.direction == DirectRequestDirection::Outgoing
                })
                .ok_or("unbekannte oder lokal geloeschte Direktanfrage")?;
            (
                tombstone.request.clone(),
                tombstone.contact_id.clone().ok_or("Kontaktbezug fehlt")?,
            )
        }
    };
    let contact = state
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .cloned()
        .ok_or("Direktkontakt fehlt")?;
    Ok((request, contact, local_identity(&state.identity)))
}

fn relay_event(
    auth: &Arc<Mutex<ShareAuthState>>,
    request_id: DirectRequestId,
    route: DirectRoute,
    outcome: DirectRouteOutcome,
    now: i64,
) -> Result<DirectSignalEvent, String> {
    if outcome == DirectRouteOutcome::LegacyForwarded && route != DirectRoute::Request {
        return Err("Legacy-Relay-ACK ist nur fuer Direktanfragen erlaubt".into());
    }
    let envelope = envelope_kind(route);
    let state = auth.lock().map_err(|_| "Share-State gesperrt")?;
    let entry = find_entry(&state.direct_requests, &request_id)?;
    if !has_outbox(entry, envelope) {
        return Err("Relay-ACK passt zu keiner lokalen Outbox".into());
    }
    Ok(DirectSignalEvent::RelayAcknowledged {
        request_id,
        envelope,
        outcome: match outcome {
            DirectRouteOutcome::Forwarded => DirectRelayOutcome::Forwarded,
            DirectRouteOutcome::LegacyForwarded => DirectRelayOutcome::LegacyForwarded,
            DirectRouteOutcome::TargetOffline => DirectRelayOutcome::TargetOffline,
        },
        at: now,
    })
}

fn find_entry<'a>(
    entries: &'a [DirectRequestEntry],
    request_id: &DirectRequestId,
) -> Result<&'a DirectRequestEntry, String> {
    entries
        .iter()
        .find(|entry| entry.record.request.request_id == *request_id)
        .ok_or_else(|| "unbekannte Direktanfrage".into())
}

fn has_outbox(entry: &DirectRequestEntry, kind: DirectEnvelopeKind) -> bool {
    match (entry.direction, kind) {
        (DirectRequestDirection::Outgoing, DirectEnvelopeKind::Request) => true,
        (DirectRequestDirection::Incoming, DirectEnvelopeKind::RequestReceipt) => {
            entry.request_receipt.is_some()
        }
        (DirectRequestDirection::Incoming, DirectEnvelopeKind::Decision) => {
            entry.decision.is_some()
        }
        (DirectRequestDirection::Outgoing, DirectEnvelopeKind::DecisionReceipt) => {
            entry.decision_receipt.is_some()
        }
        _ => false,
    }
}

fn envelope_kind(route: DirectRoute) -> DirectEnvelopeKind {
    match route {
        DirectRoute::Request => DirectEnvelopeKind::Request,
        DirectRoute::RequestReceipt => DirectEnvelopeKind::RequestReceipt,
        DirectRoute::Decision => DirectEnvelopeKind::Decision,
        DirectRoute::DecisionReceipt => DirectEnvelopeKind::DecisionReceipt,
    }
}

fn contact_secret(contact: &DirectContact) -> Result<Vec<u8>, String> {
    ShareProfiles::direct_secret_checked(contact)?.ok_or_else(|| "Direkt-Secret fehlt".to_string())
}

fn local_identity(identity: &super::identity::ShareIdentity) -> DirectPeerIdentity {
    DirectPeerIdentity {
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        node_id: identity.node_id.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
    }
}
