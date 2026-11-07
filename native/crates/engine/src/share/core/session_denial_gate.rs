//! Which peers learn why this host rejected their file session (R-ZUGANG-02,
//! review 2026-11-07). The class of a rejection (R3, `session_denial.rs`)
//! carries this host's device name and names its state (Direct access off,
//! code replaced, room left or member not admitted). Only a peer this host
//! already relates to receives it:
//!
//! * its TLS node is pinned by a Direct grant, a Direct contact or a room
//!   member of this host in any state (stale code, suspended, blocked,
//!   pending), with the same device id, or
//! * its hello carries a session proof that verifies with this host's current
//!   Direct secret or with the secret of the room it names (a new device that
//!   holds the code or the room secret).
//!
//! Every hello also has to name its key as its node (`public_key ==
//! node_id`, true for every se identity, `identity.rs`), so the device id and
//! key a rejection records (`peer_denials.rs`) are those of the TLS identity.
//! Any other peer receives the unclassified rejection without this host's
//! name; the host log keeps the former detail. Clients read a missing class
//! as before (`client_rejection`), so the wire stays unchanged.
use std::io;
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use super::super::core::{eio, verify_hmac};
use super::super::profiles::ShareProfiles;
use super::super::types::ShareAuthState;
use super::super::wire::PeerHello;
use super::denial::denial_of;
use super::session_payload;

/// `error` of a rejected hello, without its class when the peer may not
/// learn it.
pub(super) fn scoped(
    error: io::Error,
    hello: &PeerHello,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> io::Error {
    if denial_of(&error).is_none() {
        return error;
    }
    // A locked state discloses nothing (fail closed).
    let disclose = auth
        .lock()
        .map(|state| may_disclose(&state, hello))
        .unwrap_or(false);
    if disclose {
        error
    } else {
        eio(format!(
            "{error} (Gegenstelle ohne Kopplung und ohne gültigen Nachweis)"
        ))
    }
}

/// Whether the rejected peer of `hello` relates to this host.
pub(super) fn may_disclose(state: &ShareAuthState, hello: &PeerHello) -> bool {
    !hello.node_id.is_empty()
        && hello.public_key == hello.node_id
        && (pinned(state, hello) || proven(state, hello))
}

fn pinned(state: &ShareAuthState, hello: &PeerHello) -> bool {
    let (device, key, node) = (&hello.device_id, &hello.public_key, &hello.node_id);
    let grant = state.direct_grants.iter().any(|grant| {
        &grant.device_id == device && &grant.public_key == key && grant.pins_node(node)
    });
    let member = state.rooms.iter().any(|room| {
        room.members.iter().any(|member| {
            &member.device_id == device && &member.public_key == key && &member.node_id == node
        })
    });
    grant || member || contact_pins(state, hello)
}

/// A Direct contact pinning the peer's node. A contact that does not know the
/// peer's device id yet counts only while no other relation names that device
/// id for another node, so a contact cannot lend the peer a foreign name.
fn contact_pins(state: &ShareAuthState, hello: &PeerHello) -> bool {
    let (device, key, node) = (&hello.device_id, &hello.public_key, &hello.node_id);
    let mut unnamed = false;
    for contact in &state.direct_contacts {
        let contact_key = contact
            .remote_public_key
            .as_deref()
            .or(contact.accepted_public_key.as_deref())
            .unwrap_or_default();
        let pins = if contact.expected_node_id.is_empty() {
            contact_key == key.as_str()
        } else {
            &contact.expected_node_id == node
        };
        if !pins {
            continue;
        }
        match contact.remote_device_id.as_deref() {
            Some(remote) if remote == device.as_str() => return true,
            Some(_) => {}
            None => unnamed = true,
        }
    }
    unnamed && !device_named_elsewhere(state, hello)
}

fn device_named_elsewhere(state: &ShareAuthState, hello: &PeerHello) -> bool {
    let (device, node) = (&hello.device_id, &hello.node_id);
    state
        .direct_grants
        .iter()
        .any(|grant| &grant.device_id == device && !grant.pins_node(node))
        || state.direct_contacts.iter().any(|contact| {
            contact.remote_device_id.as_deref() == Some(device.as_str())
                && !contact.expected_node_id.is_empty()
                && &contact.expected_node_id != node
        })
        || state.rooms.iter().any(|room| {
            room.members
                .iter()
                .any(|member| &member.device_id == device && &member.node_id != node)
        })
}

/// The hello's session proof verifies with the secret of the relation it
/// names: the current Direct secret, or the stored secret of that room.
fn proven(state: &ShareAuthState, hello: &PeerHello) -> bool {
    let payload = |kind: &str| {
        session_payload(
            kind,
            &hello.relation_id,
            &hello.device_id,
            &state.identity.device_id,
            &hello.node_id,
            &state.identity.node_id,
            &hello.session_nonce,
        )
    };
    match hello.relation_kind.as_str() {
        "direct" => {
            hello.relation_id == state.identity.direct_lookup_id
                && !state.direct_secret.is_empty()
                && verify_hmac(
                    &state.direct_secret,
                    &payload("direct"),
                    &hello.session_proof,
                )
        }
        "room" => state
            .rooms
            .iter()
            .find(|room| room.room_id == hello.relation_id)
            .and_then(|room| ShareProfiles::room_secret_checked(room).ok().flatten())
            .map(Zeroizing::new)
            .is_some_and(|secret| {
                !secret.is_empty() && verify_hmac(&secret, &payload("room"), &hello.session_proof)
            }),
        _ => false,
    }
}

#[cfg(test)]
#[path = "session_denial_gate_tests.rs"]
mod tests;
