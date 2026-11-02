//! Verification of signaling messages before they reach the daemon: relation
//! HMAC, device signature with downgrade protection (B03), node binding (S17)
//! and replay protection until expiry (S16).
#[cfg(test)]
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::core::{now_secs, presence_payload, verify_hmac};
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::signal_presence::PresenceSignature;
use super::types::{DirectContact, PeerPresence, ShareAuthState, ShareEvent};
use super::wire::SrvMsg;

#[path = "signal_auth_replay.rs"]
mod replay;
#[cfg(test)]
pub(super) use replay::remember_replay;
use replay::{remember_presence, replay_key, signature_seen};

/// A probe must preserve the immediate, cross-relation downgrade marker too.
pub(super) fn peer_signature_seen(state: &ShareAuthState, presence: &PeerPresence) -> bool {
    signature_seen(&state.seen_nonces, presence)
}

/// Handles one legacy signaling line; returns whether it was a pong. With
/// `tracked_direct` negotiated, legacy decisions are dropped: they carry no
/// signed decision and must not override verified ones (S30, S46).
pub(super) fn handle_server_msg(
    line: &str,
    auth: &Arc<Mutex<ShareAuthState>>,
    events: &crossbeam_channel::Sender<ShareEvent>,
    tracked_direct: bool,
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
                    presence: presence.with_local_fingerprint(),
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
                    presence: presence.with_local_fingerprint(),
                });
            }
        }
        SrvMsg::DirectAccessAccepted { .. } if tracked_direct => {
            let _ = events.send(ShareEvent::Status(
                "Legacy-Entscheidung verworfen: der Server spricht tracked_direct".into(),
            ));
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
                    presence: presence.map(PeerPresence::with_local_fingerprint),
                    msg,
                });
            }
        }
        SrvMsg::RoomRoster { room_id, members } => {
            let valid: Vec<_> = members
                .into_iter()
                .filter(|presence| verify_room_presence(&room_id, presence, auth))
                .map(PeerPresence::with_local_fingerprint)
                .collect();
            let _ = events.send(ShareEvent::RoomRoster {
                room_id,
                members: valid,
            });
        }
        SrvMsg::RoomJoined { room_id, presence } => {
            if verify_room_presence(&room_id, &presence, auth) {
                let _ = events.send(ShareEvent::RoomJoined {
                    room_id,
                    presence: presence.with_local_fingerprint(),
                });
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

/// The device signature a receiver demands: a valid one always passes, an
/// invalid one never; an unsigned presence only from a peer that never signed
/// (no downgrade, B03) and with the node bound to its key (S17).
fn accepts_signature(presence: &PeerPresence, signed_before: bool) -> bool {
    match presence.signature_state() {
        PresenceSignature::Valid => true,
        PresenceSignature::Invalid => false,
        PresenceSignature::Unsigned => !signed_before && presence.node_id == presence.public_key,
    }
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
    let replay_key = replay_key(
        presence.expires_at,
        &format!(
            "direct-request:{lookup_id}:{}:{}",
            presence.device_id, presence.nonce
        ),
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
    let signed_before = state.direct_contacts.iter().any(|contact| {
        contact.relation.signed_presence
            && (contact.remote_public_key.as_deref() == Some(presence.public_key.as_str())
                || !contact.expected_node_id.is_empty()
                    && contact.expected_node_id == presence.node_id)
    });
    if !verify_hmac(&state.direct_secret, &payload, &presence.proof)
        || !accepts_signature(
            presence,
            signed_before || signature_seen(&state.seen_nonces, presence),
        )
    {
        return false;
    }
    if !remember_presence(&mut state.seen_nonces, replay_key, now, presence) {
        return false;
    }
    if presence.is_signed() {
        for contact in &mut state.direct_contacts {
            if contact.remote_public_key.as_deref() == Some(presence.public_key.as_str())
                || !contact.expected_node_id.is_empty()
                    && contact.expected_node_id == presence.node_id
            {
                contact.relation.signed_presence = true;
            }
        }
    }
    true
}

fn verify_direct_access_accepted(
    lookup_id: &str,
    requester_device_id: &str,
    accepted: bool,
    presence: Option<&PeerPresence>,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    if presence
        .is_none_or(|presence| !presence.matches_legacy_decision(requester_device_id, accepted))
    {
        return false;
    }
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
    let now = now_secs();
    if !presence.is_current_at(now)
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
    if !pins_match(contact, presence) {
        return false;
    }
    let signed_before = contact.relation.signed_presence;
    let Some(secret) = secret_for(contact) else {
        return false;
    };
    let replay_key = replay_key(
        presence.expires_at,
        &format!(
            "direct-accepted:{lookup_id}:{}:{}",
            presence.device_id, presence.nonce
        ),
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
    if !verify_hmac(&secret, &payload, &presence.proof)
        || !accepts_signature(
            presence,
            signed_before || signature_seen(&state.seen_nonces, presence),
        )
    {
        return false;
    }
    if !remember_presence(&mut state.seen_nonces, replay_key, now, presence) {
        return false;
    }
    if presence.is_signed() {
        for contact in &mut state.direct_contacts {
            if contact.lookup_id == lookup_id {
                contact.relation.signed_presence = true;
            }
        }
    }
    true
}

/// The code pins (fingerprint, node) and every identity value this contact
/// already learned; pinned values are never replaced by a presence (S29, S30).
fn pins_match(contact: &DirectContact, presence: &PeerPresence) -> bool {
    fingerprint_matches(&presence.public_key, &contact.expected_fingerprint)
        && (contact.expected_node_id.trim().is_empty()
            || contact.expected_node_id == presence.node_id)
        && contact
            .remote_public_key
            .as_deref()
            .is_none_or(|key| key == presence.public_key)
        && contact
            .remote_device_id
            .as_deref()
            .is_none_or(|id| id == presence.device_id)
        && contact
            .accepted_public_key
            .as_deref()
            .is_none_or(|key| key == presence.public_key)
}

fn verify_direct_presence(
    lookup_id: &str,
    presence: &PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    let now = now_secs();
    if !presence.is_current_at(now)
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
    if !pins_match(contact, presence) {
        return false;
    }
    let signed_before = contact.relation.signed_presence;
    let Some(secret) = ShareProfiles::direct_secret(contact) else {
        return false;
    };
    let replay_key = replay_key(
        presence.expires_at,
        &format!("direct:{lookup_id}:{}", presence.nonce),
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
    if !verify_hmac(&secret, &payload, &presence.proof)
        || !accepts_signature(
            presence,
            signed_before || signature_seen(&state.seen_nonces, presence),
        )
    {
        return false;
    }
    if !remember_presence(&mut state.seen_nonces, replay_key, now, presence) {
        return false;
    }
    if presence.is_signed() {
        for contact in &mut state.direct_contacts {
            if contact.lookup_id == lookup_id {
                contact.relation.signed_presence = true;
            }
        }
    }
    true
}

fn verify_room_presence(
    room_id: &str,
    presence: &PeerPresence,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> bool {
    let now = now_secs();
    if !presence.is_current_at(now) || presence.kind != "room" || presence.relation_id != room_id {
        return false;
    }
    let mut state = match auth.lock() {
        Ok(state) => state,
        Err(_) => return false,
    };
    let Some(room) = state.rooms.iter().find(|room| room.room_id == room_id) else {
        return false;
    };
    if room.members.iter().any(|member| {
        member.device_id == presence.device_id
            && (member.public_key != presence.public_key
                || !member.node_id.is_empty() && member.node_id != presence.node_id)
    }) {
        return false;
    }
    let signed_before = room.members.iter().any(|member| {
        member.relation.signed_presence
            && (member.device_id == presence.device_id
                || member.public_key == presence.public_key
                || !member.node_id.is_empty() && member.node_id == presence.node_id)
    });
    let Some(secret) = ShareProfiles::room_secret(room) else {
        return false;
    };
    let replay_key = replay_key(
        presence.expires_at,
        &format!("room:{room_id}:{}:{}", presence.device_id, presence.nonce),
    );
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
    if !verify_hmac(&secret, &payload, &presence.proof)
        || !accepts_signature(
            presence,
            signed_before || signature_seen(&state.seen_nonces, presence),
        )
    {
        return false;
    }
    if !remember_presence(&mut state.seen_nonces, replay_key, now, presence) {
        return false;
    }
    if presence.is_signed() && presence.device_id != state.identity.device_id {
        if let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) {
            // Pin the first valid signature immediately. The daemon may not
            // yet have folded this roster event when the next one arrives.
            room.upsert_member_from_presence(presence.clone().with_local_fingerprint(), now);
        }
    }
    true
}

#[cfg(test)]
pub(super) fn remember_nonce(seen: &mut HashSet<String>, key: String) {
    remember_replay(seen, key, now_secs());
}
