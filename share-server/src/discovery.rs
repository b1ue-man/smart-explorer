use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::discovery_state::{
    finish_exchange_locked, offer_capacity_available, prepare_exchange_locked, prune_locked,
    remove_offer_locked, route_packet_locked, same_public_offer, unix_seconds, valid_payload,
    valid_text, validate_offer_request, DiscoveryOffer, PacketRoute, MAX_DISCOVERY_LIST_ENTRIES,
    MAX_EXCHANGE_ID_BYTES, MAX_OFFER_ID_BYTES, MAX_SERVER_LEASE,
};
use super::protocol::{
    DiscoveryAdvertisement, DiscoveryOfferRequest, PairingCloseReason, PairingPacketKind,
};
use super::state::{lock_state, State};
use super::{send, Out, Writer};

pub(super) const CAPABILITY: &str = "discovery_exchange_v1";

pub(super) fn publish(
    client_id: u64,
    origin: &Writer,
    request: DiscoveryOfferRequest,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(client_id, origin, state) {
        return;
    }
    if let Err(message) = validate_offer_request(&request) {
        send_error(origin, message);
        return;
    }

    let now = Instant::now();
    let lease = Duration::from_secs(u64::from(request.lease_secs)).min(MAX_SERVER_LEASE);
    let expires_at = unix_seconds().saturating_add(lease.as_secs() as i64);
    let (result, notifications) = {
        let mut state = lock_state(state);
        let notifications = prune_locked(&mut state, now);
        let result = upsert_offer_locked(&mut state, client_id, request, now, lease, expires_at);
        (result, notifications)
    };
    send_all(notifications);
    match result {
        Ok(advertisement) => {
            send(origin, &Out::DiscoveryPublished { advertisement });
        }
        Err(message) => send_error(origin, message),
    }
}

pub(super) fn unpublish(
    client_id: u64,
    origin: &Writer,
    offer_id: &str,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(client_id, origin, state) {
        return;
    }
    if !valid_text(offer_id, MAX_OFFER_ID_BYTES, false) {
        send_error(origin, "invalid offer id");
        return;
    }
    let notifications = {
        let mut state = lock_state(state);
        let mut notifications = prune_locked(&mut state, Instant::now());
        if let Some(discovery_id) = state
            .discovery_offer_index
            .get(&(client_id, offer_id.to_string()))
            .cloned()
        {
            notifications.extend(remove_offer_locked(
                &mut state,
                &discovery_id,
                PairingCloseReason::OfferWithdrawn,
                None,
            ));
        }
        notifications
    };
    send_all(notifications);
}

pub(super) fn list(client_id: u64, origin: &Writer, state: &Arc<Mutex<State>>) {
    if !require_capability(client_id, origin, state) {
        return;
    }
    let (mut advertisements, notifications) = {
        let mut state = lock_state(state);
        let notifications = prune_locked(&mut state, Instant::now());
        let advertisements = state
            .discovery_offers
            .values()
            .filter(|offer| offer.owner_id != client_id)
            .map(|offer| offer.advertisement.clone())
            .take(MAX_DISCOVERY_LIST_ENTRIES)
            .collect::<Vec<_>>();
        (advertisements, notifications)
    };
    send_all(notifications);
    advertisements.sort_by(|left, right| {
        left.expires_at
            .cmp(&right.expires_at)
            .then_with(|| left.discovery_id.cmp(&right.discovery_id))
    });
    send(origin, &Out::DiscoveryList { advertisements });
}

pub(super) fn start_pairing(
    connector_id: u64,
    origin: &Writer,
    discovery_id: &str,
    exchange_id: &str,
    payload: String,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(connector_id, origin, state) {
        return;
    }
    if !valid_text(discovery_id, MAX_OFFER_ID_BYTES, false)
        || !valid_text(exchange_id, MAX_EXCHANGE_ID_BYTES, false)
        || !valid_payload(&payload)
    {
        send_error(origin, "invalid pairing start");
        return;
    }
    let now = Instant::now();
    let (result, notifications) = {
        let mut state = lock_state(state);
        let notifications = prune_locked(&mut state, now);
        let result = prepare_exchange_locked(
            &mut state,
            connector_id,
            discovery_id,
            exchange_id,
            payload.len(),
            now,
        );
        (result, notifications)
    };
    send_all(notifications);
    let publisher = match result {
        Ok(publisher) => publisher,
        Err(message) => {
            send_error(origin, message);
            return;
        }
    };
    if !send(
        origin,
        &Out::PairingOpened {
            exchange_id: exchange_id.to_string(),
            discovery_id: discovery_id.to_string(),
        },
    ) || !send(
        &publisher,
        &Out::PairingStarted {
            exchange_id: exchange_id.to_string(),
            discovery_id: discovery_id.to_string(),
            payload,
        },
    ) {
        finish_exchange(state, exchange_id, PairingCloseReason::TargetUnavailable);
    }
}

pub(super) fn pairing_packet(
    client_id: u64,
    origin: &Writer,
    exchange_id: &str,
    kind: PairingPacketKind,
    payload: String,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(client_id, origin, state) {
        return;
    }
    if !valid_text(exchange_id, MAX_EXCHANGE_ID_BYTES, false) || !valid_payload(&payload) {
        send_error(origin, "invalid pairing packet");
        return;
    }
    let (route, notifications) = {
        let mut state = lock_state(state);
        let notifications = prune_locked(&mut state, Instant::now());
        let route = route_packet_locked(&mut state, client_id, exchange_id, kind, payload.len());
        (route, notifications)
    };
    send_all(notifications);
    match route {
        PacketRoute::Forward { target, completes } => {
            if !send(
                &target,
                &Out::PairingPacket {
                    exchange_id: exchange_id.to_string(),
                    kind,
                    payload,
                },
            ) {
                finish_exchange(state, exchange_id, PairingCloseReason::TargetUnavailable);
            } else if completes {
                finish_exchange(state, exchange_id, PairingCloseReason::Completed);
            }
        }
        PacketRoute::Reject { message, close } => {
            send_error(origin, message);
            if close {
                finish_exchange(state, exchange_id, PairingCloseReason::ProtocolError);
            }
        }
    }
}

pub(super) fn cancel_pairing(
    client_id: u64,
    origin: &Writer,
    exchange_id: &str,
    state: &Arc<Mutex<State>>,
) {
    if !require_capability(client_id, origin, state) {
        return;
    }
    if !valid_text(exchange_id, MAX_EXCHANGE_ID_BYTES, false) {
        send_error(origin, "invalid pairing exchange id");
        return;
    }
    let result = {
        let mut state = lock_state(state);
        let mut notifications = prune_locked(&mut state, Instant::now());
        let allowed = state
            .discovery_exchanges
            .get(exchange_id)
            .is_some_and(|exchange| {
                exchange.publisher_id == client_id || exchange.connector_id == client_id
            });
        if allowed {
            notifications.extend(finish_exchange_locked(
                &mut state,
                exchange_id,
                PairingCloseReason::Cancelled,
                None,
            ));
            Ok(notifications)
        } else {
            Err(notifications)
        }
    };
    match result {
        Ok(notifications) => send_all(notifications),
        Err(notifications) => {
            send_all(notifications);
            send_error(origin, "pairing exchange not found for this client");
        }
    }
}

pub(super) fn prune_expired(state: &Arc<Mutex<State>>) {
    let notifications = {
        let mut state = lock_state(state);
        prune_locked(&mut state, Instant::now())
    };
    send_all(notifications);
}

pub(super) fn cleanup_client_locked(state: &mut State, client_id: u64) -> Vec<(Writer, Out)> {
    let offer_ids = state
        .discovery_offers
        .iter()
        .filter_map(|(id, offer)| (offer.owner_id == client_id).then(|| id.clone()))
        .collect::<Vec<_>>();
    let mut notifications = Vec::new();
    for discovery_id in offer_ids {
        notifications.extend(remove_offer_locked(
            state,
            &discovery_id,
            PairingCloseReason::PeerDisconnected,
            Some(client_id),
        ));
    }
    let exchange_ids = state
        .discovery_exchanges
        .iter()
        .filter_map(|(id, exchange)| {
            (exchange.publisher_id == client_id || exchange.connector_id == client_id)
                .then(|| id.clone())
        })
        .collect::<Vec<_>>();
    for exchange_id in exchange_ids {
        notifications.extend(finish_exchange_locked(
            state,
            &exchange_id,
            PairingCloseReason::PeerDisconnected,
            Some(client_id),
        ));
    }
    notifications
}

fn upsert_offer_locked(
    state: &mut State,
    client_id: u64,
    request: DiscoveryOfferRequest,
    now: Instant,
    lease: Duration,
    expires_at: i64,
) -> Result<DiscoveryAdvertisement, &'static str> {
    let key = (client_id, request.offer_id.clone());
    if let Some(discovery_id) = state.discovery_offer_index.get(&key).cloned() {
        let Some(existing) = state.discovery_offers.get_mut(&discovery_id) else {
            state.discovery_offer_index.remove(&key);
            return Err("discovery offer index was stale");
        };
        if !same_public_offer(&existing.advertisement, &request) {
            return Err("offer id is already bound to different public metadata");
        }
        existing.deadline = now + lease;
        existing.advertisement.expires_at = expires_at;
        return Ok(existing.advertisement.clone());
    }

    offer_capacity_available(state, client_id)?;
    let next_id = state
        .next_discovery_id
        .checked_add(1)
        .ok_or("server discovery id space exhausted")?;
    state.next_discovery_id = next_id;
    let discovery_id = format!("discovery-{next_id:016x}");
    let advertisement = DiscoveryAdvertisement {
        discovery_id: discovery_id.clone(),
        offer_id: request.offer_id,
        kind: request.kind,
        display_alias: request.display_alias,
        suite: request.suite,
        version: request.version,
        expires_at,
    };
    state
        .discovery_offer_index
        .insert(key, discovery_id.clone());
    state.discovery_offers.insert(
        discovery_id,
        DiscoveryOffer::new(client_id, advertisement.clone(), now + lease),
    );
    Ok(advertisement)
}

fn finish_exchange(state: &Arc<Mutex<State>>, exchange_id: &str, reason: PairingCloseReason) {
    let notifications = {
        let mut state = lock_state(state);
        finish_exchange_locked(&mut state, exchange_id, reason, None)
    };
    send_all(notifications);
}

fn require_capability(client_id: u64, origin: &Writer, state: &Arc<Mutex<State>>) -> bool {
    let supported = lock_state(state)
        .clients
        .get(&client_id)
        .is_some_and(|client| client.capabilities.contains(CAPABILITY));
    if !supported {
        send_error(
            origin,
            "discovery_exchange_v1 capability was not negotiated",
        );
    }
    supported
}

fn send_error(origin: &Writer, message: &str) {
    send(
        origin,
        &Out::Error {
            scope: "discovery".into(),
            msg: message.to_string(),
        },
    );
}

fn send_all(notifications: Vec<(Writer, Out)>) {
    for (writer, message) in notifications {
        send(&writer, &message);
    }
}
