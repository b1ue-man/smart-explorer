//! `ShareStatus` JSON (api.md §5) from the last worker snapshot and the
//! client state, as a pure projection (no I/O).
use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};

use super::share_exec::{provider_json, targets_json};
use crate::share::discovery_state::{
    DiscoveryExchangeState, DiscoveryPublishTarget, DiscoveryUiKind, DiscoveryUiState,
};
use crate::share::lifecycle_view::{request_views, RequestView};
use crate::share::{
    DirectContact, DirectDecisionState, ExecProviderStatus, PeerOpenTarget, ShareExportConfig,
    ShareIdentity, ShareProfiles, ShareStatus,
};

/// Worker facts of the last successful snapshot.
#[derive(Clone, Debug, Default)]
pub(super) struct WorkerFacts {
    pub running: bool,
    pub connected: bool,
    pub relay_url: String,
    pub last_error: Option<String>,
    pub lan_presence: String,
}

impl WorkerFacts {
    pub(super) fn from_snapshot(snapshot: &crate::daemon::ShareWorkerSnapshot) -> Self {
        Self {
            running: snapshot.running,
            connected: snapshot.connected,
            relay_url: snapshot.relay_url.clone(),
            last_error: snapshot.last_error.clone(),
            lan_presence: snapshot.lan.presence.label(),
        }
    }
}

pub(super) struct StatusInput<'a> {
    pub worker: Option<&'a WorkerFacts>,
    pub profiles: &'a ShareProfiles,
    pub identity: Option<&'a ShareIdentity>,
    pub server: &'a str,
    pub poll_error: Option<&'a str>,
    pub discovery: &'a DiscoveryUiState,
    /// Alias per offer target key (`direct` or a room profile id).
    pub offer_aliases: &'a BTreeMap<String, String>,
    pub last_exchange: Option<&'a str>,
    pub notices: &'a VecDeque<String>,
    pub now_secs: i64,
    /// This phone as exec host (`execProvider`).
    pub exec_provider: &'a ExecProviderStatus,
}

pub(super) fn status_code(status: &ShareStatus) -> &'static str {
    match status {
        ShareStatus::Offline => "offline",
        ShareStatus::Waiting => "waiting",
        ShareStatus::WaitingForAccess => "waitingForAccess",
        ShareStatus::Available => "available",
        ShareStatus::Connecting => "connecting",
        ShareStatus::Connected => "connected",
        ShareStatus::ConnectedDirect => "connectedDirect",
        ShareStatus::ConnectedRelay => "connectedRelay",
        ShareStatus::Failed(_) => "failed",
        ShareStatus::IdentityConflict => "identityConflict",
    }
}

fn reachable(status: &ShareStatus) -> bool {
    matches!(
        status,
        ShareStatus::Available
            | ShareStatus::Connected
            | ShareStatus::ConnectedDirect
            | ShareStatus::ConnectedRelay
    )
}

fn seen_on_lan(contact: &DirectContact, now_secs: i64) -> bool {
    contact.lan_seen_at.is_some_and(|seen| {
        now_secs.saturating_sub(seen) <= crate::share::LAN_PRESENCE_TTL_SECS
            && !contact.lan_candidates.is_empty()
    })
}

fn device_json(contact: &DirectContact, now_secs: i64) -> Value {
    let lan = seen_on_lan(contact, now_secs);
    let target = PeerOpenTarget::Direct {
        contact_id: contact.id.clone(),
    };
    json!({
        "contactId": contact.id,
        "name": contact.display_name,
        "status": status_code(&contact.status),
        "statusText": contact.status.label(),
        "online": reachable(&contact.status) || lan,
        "location": target.endpoint_prefix(),
        "lan": lan,
    })
}

fn rooms_json(profiles: &ShareProfiles) -> Vec<Value> {
    profiles
        .rooms
        .iter()
        .map(|room| {
            let members: Vec<Value> = room
                .members
                .iter()
                .map(|member| {
                    let target = PeerOpenTarget::RoomDevice {
                        room_id: room.id.clone(),
                        device_id: member.device_id.clone(),
                    };
                    json!({
                        "deviceId": member.device_id,
                        "name": member.device_name,
                        "status": member.status.label(),
                        "location": target.endpoint_prefix(),
                        "blocked": member.blocked,
                    })
                })
                .collect();
            json!({
                "profileId": room.id,
                "roomId": room.room_id,
                "name": room.name,
                "status": room.status.label(),
                "autoJoin": room.auto_join,
                "location": Value::Null,
                "members": members,
            })
        })
        .collect()
}

pub(super) fn decision_label(state: DirectDecisionState) -> &'static str {
    match state {
        DirectDecisionState::Pending => "Offen",
        DirectDecisionState::Accepted => "Angenommen",
        DirectDecisionState::Rejected => "Abgelehnt",
        DirectDecisionState::Revoked => "Widerrufen",
        DirectDecisionState::Failed => "Fehlgeschlagen",
        DirectDecisionState::Expired => "Abgelaufen",
    }
}

fn request_json(profiles: &ShareProfiles, view: &RequestView) -> Value {
    let entry = profiles.direct_request(&view.request_id);
    let mut state_text = decision_label(view.decision).to_string();
    if view.identity_conflict {
        state_text.push_str(" · Identitätskonflikt: Annehmen gesperrt");
    }
    json!({
        "requestId": view.request_id.as_str(),
        "contactId": entry.and_then(|entry| entry.contact_id.clone()),
        "name": view.peer_name,
        "stateText": state_text,
        "canAccept": view.can_accept,
        "canReject": view.can_decide,
        "canRetry": view.can_retry,
        "canDelete": view.can_delete,
        "message": entry.and_then(|entry| entry.record.request.message.clone()),
        "timeMs": entry
            .map(|entry| entry.record.request.created_at.saturating_mul(1000))
            .unwrap_or(0),
    })
}

fn exports_json(config: &ShareExportConfig) -> Vec<Value> {
    config
        .roots
        .iter()
        .map(|root| json!({ "label": root.label, "path": root.path }))
        .collect()
}

fn target_key(target: &DiscoveryPublishTarget) -> String {
    match target {
        DiscoveryPublishTarget::Direct => "direct".to_string(),
        DiscoveryPublishTarget::Room { room_id, .. } => room_id.clone(),
    }
}

fn discovery_json(input: &StatusInput<'_>) -> Value {
    let discovery = input.discovery;
    let offer = discovery
        .active_offers
        .iter()
        .min_by_key(|offer| !matches!(offer.target, DiscoveryPublishTarget::Direct))
        .map(|offer| {
            let key = target_key(&offer.target);
            let alias = match &offer.target {
                DiscoveryPublishTarget::Room { room_name, .. } => room_name.clone(),
                DiscoveryPublishTarget::Direct => {
                    input.offer_aliases.get(&key).cloned().unwrap_or_default()
                }
            };
            json!({
                "offerId": offer.offer_id,
                "target": key,
                "alias": alias,
                "untilMs": offer.expires_at.saturating_mul(1000),
            })
        });
    let advertisements: Vec<Value> = discovery
        .entries
        .iter()
        .map(|entry| {
            json!({
                "discoveryId": entry.discovery_id,
                "kind": match entry.kind {
                    DiscoveryUiKind::Direct => "direct",
                    DiscoveryUiKind::Room => "room",
                },
                "alias": entry.display_alias,
                "expiresMs": entry.expires_at.saturating_mul(1000),
                "compatible": entry.compatibility.can_connect(),
            })
        })
        .collect();
    let exchange = input.last_exchange.and_then(|exchange_id| {
        let record = discovery.exchanges.get(exchange_id)?;
        let state = match &record.state {
            DiscoveryExchangeState::Exchanging | DiscoveryExchangeState::Cancelling => "running",
            DiscoveryExchangeState::Complete(_) => "done",
            DiscoveryExchangeState::Failed(_) => "failed",
            DiscoveryExchangeState::Cancelled => "canceled",
        };
        Some(json!({
            "exchangeId": exchange_id,
            "state": state,
            "message": record.state.label(),
        }))
    });
    json!({
        "offer": offer,
        "advertisements": advertisements,
        "exchange": exchange,
    })
}

pub(super) fn status_json(input: &StatusInput<'_>) -> Value {
    let profiles = input.profiles;
    let worker = input.worker.cloned().unwrap_or_default();
    let identity = match input.identity {
        Some(identity) => json!({
            "deviceId": identity.device_id,
            "deviceName": identity.device_name,
            "fingerprint": identity.fingerprint,
            "directCode": identity.direct_code(),
        }),
        None => json!({ "deviceId": "", "deviceName": "", "fingerprint": "", "directCode": "" }),
    };
    let devices: Vec<Value> = profiles
        .direct_contacts
        .iter()
        .map(|contact| device_json(contact, input.now_secs))
        .collect();
    let (incoming, outgoing) = request_views(profiles, input.now_secs);
    let incoming: Vec<Value> = incoming
        .iter()
        .map(|view| request_json(profiles, view))
        .collect();
    let outgoing: Vec<Value> = outgoing
        .iter()
        .map(|view| request_json(profiles, view))
        .collect();
    let room_exports: serde_json::Map<String, Value> = profiles
        .rooms
        .iter()
        .map(|room| (room.id.clone(), Value::Array(exports_json(&room.exports))))
        .collect();
    let removed: Vec<Value> = profiles
        .removed_direct_peers
        .iter()
        .map(|peer| json!({ "deviceId": peer.device_id, "name": peer.device_name }))
        .collect();
    let last_error = input
        .poll_error
        .map(str::to_string)
        .or_else(|| worker.last_error.clone());
    let server = input.server.trim();
    let lan_presence = if worker.lan_presence.is_empty() {
        "aus".to_string()
    } else {
        worker.lan_presence.clone()
    };
    let notices: Vec<&String> = input.notices.iter().collect();
    json!({
        "running": worker.running,
        "connected": worker.connected,
        "relayUrl": (!worker.relay_url.is_empty()).then(|| worker.relay_url.clone()),
        "lastError": last_error,
        "server": (!server.is_empty()).then_some(server),
        "lanPresence": lan_presence,
        "identity": identity,
        "devices": devices,
        "execProvider": provider_json(input.exec_provider),
        "execTargets": targets_json(profiles),
        "rooms": rooms_json(profiles),
        "incoming": incoming,
        "outgoing": outgoing,
        "exports": {
            "direct": exports_json(&profiles.default_direct_exports),
            "rooms": room_exports,
        },
        "discovery": discovery_json(input),
        "removedDevices": removed,
        "notices": notices,
    })
}

/// Incoming requests that still wait for this device's decision.
pub(super) fn open_incoming(profiles: &ShareProfiles, now_secs: i64) -> Vec<String> {
    request_views(profiles, now_secs)
        .0
        .into_iter()
        .filter(|view| view.can_decide)
        .map(|view| view.request_id.as_str().to_string())
        .collect()
}
