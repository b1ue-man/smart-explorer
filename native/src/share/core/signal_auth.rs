use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::core::{now_secs, presence_payload, verify_hmac};
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::types::{DirectContact, PeerPresence, ShareAuthState, ShareEvent};
use super::wire::SrvMsg;

pub(super) fn handle_server_msg(
    line: &str,
    auth: &Arc<Mutex<ShareAuthState>>,
    events: &crossbeam_channel::Sender<ShareEvent>,
) -> bool {
    if line.is_empty() {
        return false;
    }
    let message: SrvMsg = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => {
            let _ = events.send(ShareEvent::Error(format!("Server-Nachricht: {error}")));
            return false;
        }
    };
    let pong = matches!(message, SrvMsg::Pong);
    match message {
        SrvMsg::HelloOk { .. } | SrvMsg::Pong => {}
        SrvMsg::DirectAvailable {
            lookup_id,
            presence,
        } => {
            if verify_direct_presence(&lookup_id, &presence, auth) {
                let _ = events.send(ShareEvent::DirectAvailable {
                    lookup_id,
                    presence,
                });
            }
        }
        SrvMsg::DirectOffline { lookup_id } => {
            let _ = events.send(ShareEvent::DirectOffline { lookup_id });
        }
        SrvMsg::DirectAccessRequest {
            lookup_id,
            presence,
        } => {
            if verify_local_direct_request(&lookup_id, &presence, auth) {
                let _ = events.send(ShareEvent::DirectAccessRequest {
                    lookup_id,
                    presence,
                });
            }
        }
        SrvMsg::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            msg,
        } => {
            if verify_direct_access_accepted(
                &lookup_id,
                &requester_device_id,
                accepted,
                presence.as_ref(),
                auth,
            ) {
                let _ = events.send(ShareEvent::DirectAccessAccepted {
                    lookup_id,
                    requester_device_id,
                    accepted,
                    presence,
                    msg,
                });
            }
        }
        SrvMsg::RoomRoster { room_id, members } => {
            let valid: Vec<_> = members
                .into_iter()
                .filter(|presence| verify_room_presence(&room_id, presence, auth))
                .collect();
            let _ = events.send(ShareEvent::RoomRoster {
                room_id,
                members: valid,
            });
        }
        SrvMsg::RoomJoined { room_id, presence } => {
            if verify_room_presence(&room_id, &presence, auth) {
                let _ = events.send(ShareEvent::RoomJoined { room_id, presence });
            }
        }
        SrvMsg::RoomLeft { room_id, device_id } => {
            let _ = events.send(ShareEvent::RoomLeft { room_id, device_id });
        }
        SrvMsg::Error { scope, msg } => {
            let _ = events.send(ShareEvent::Error(format!("{scope}: {msg}")));
        }
    }
    pong
}

pub(super) fn verify_local_direct_request(
    lookup_id: &str,
    presence: &PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    let now = now_secs();
    if super::legacy_direct_request_validation::validate_presence(lookup_id, presence, Some(now))
        .is_err()
    {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(state) => state,
        Err(_) => return false,
    };
    if lookup_id != state.identity.direct_lookup_id {
        return false;
    }
    let replay_key = format!(
        "direct-request:{lookup_id}:{}:{}",
        presence.device_id, presence.nonce
    );
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&state.direct_secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn verify_direct_access_accepted(
    lookup_id: &str,
    requester_device_id: &str,
    _accepted: bool,
    presence: Option<&PeerPresence>,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    verify_direct_access_accepted_using(
        lookup_id,
        requester_device_id,
        presence,
        auth,
        ShareProfiles::direct_secret,
    )
}

pub(super) fn verify_direct_access_accepted_using<F>(
    lookup_id: &str,
    requester_device_id: &str,
    presence: Option<&PeerPresence>,
    auth: &Arc<Mutex<ShareAuthState>>,
    secret_for: F,
) -> bool
where
    F: FnOnce(&DirectContact) -> Option<Vec<u8>>,
{
    let mut state = match auth.lock() {
        Ok(state) => state,
        Err(_) => return false,
    };
    if requester_device_id != state.identity.device_id {
        return false;
    }
    let Some(presence) = presence else {
        return false;
    };
    if !presence.is_current_at(now_secs())
        || presence.kind != "direct"
        || presence.relation_id != lookup_id
    {
        return false;
    }
    let Some(contact) = state
        .direct_contacts
        .iter()
        .find(|contact| contact.lookup_id == lookup_id)
    else {
        return false;
    };
    if !fingerprint_matches(&presence.public_key, &contact.expected_fingerprint) {
        return false;
    }
    if !contact.expected_node_id.trim().is_empty() && contact.expected_node_id != presence.node_id {
        return false;
    }
    let Some(secret) = secret_for(contact) else {
        return false;
    };
    let replay_key = format!(
        "direct-accepted:{lookup_id}:{}:{}",
        presence.device_id, presence.nonce
    );
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn verify_direct_presence(
    lookup_id: &str,
    presence: &PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    if !presence.is_current_at(now_secs())
        || presence.kind != "direct"
        || presence.relation_id != lookup_id
    {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(state) => state,
        Err(_) => return false,
    };
    let Some(contact) = state
        .direct_contacts
        .iter()
        .find(|contact| contact.lookup_id == lookup_id)
    else {
        return false;
    };
    if !fingerprint_matches(&presence.public_key, &contact.expected_fingerprint) {
        return false;
    }
    if !contact.expected_node_id.trim().is_empty() && contact.expected_node_id != presence.node_id {
        return false;
    }
    let Some(secret) = ShareProfiles::direct_secret(contact) else {
        return false;
    };
    let replay_key = format!("direct:{lookup_id}:{}", presence.nonce);
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

fn verify_room_presence(
    room_id: &str,
    presence: &PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    if !presence.is_current_at(now_secs())
        || presence.kind != "room"
        || presence.relation_id != room_id
    {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(state) => state,
        Err(_) => return false,
    };
    let Some(room) = state.rooms.iter().find(|room| room.room_id == room_id) else {
        return false;
    };
    let Some(secret) = ShareProfiles::room_secret(room) else {
        return false;
    };
    let replay_key = format!("room:{room_id}:{}:{}", presence.device_id, presence.nonce);
    if state.seen_nonces.contains(&replay_key) {
        return false;
    }
    let payload = presence_payload(
        "room",
        room_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    if !verify_hmac(&secret, &payload, &presence.proof) {
        return false;
    }
    remember_nonce(&mut state.seen_nonces, replay_key);
    true
}

pub(super) fn remember_nonce(seen: &mut HashSet<String>, key: String) {
    if seen.len() > 4096 {
        seen.clear();
    }
    seen.insert(key);
}
