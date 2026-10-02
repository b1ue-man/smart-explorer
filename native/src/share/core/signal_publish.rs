//! Presence, watches, room joins and direct requests the worker publishes
//! on the signal connection.
//!
//! After a confirmed key login (`key_login_v1`) the messages carry relation
//! access proofs (FC4): the owner of a Direct lookup leaves the hash of its
//! proof, watchers and room members show the proof itself. A proof is an
//! HMAC under the relation secret, so the server learns neither the secret
//! nor a way to derive it, and only holders of the code or room secret pass.

use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::share::backend::ShareIrohNode;
use crate::share::core::{eio, hex};
use crate::share::profiles::ShareProfiles;
use crate::share::signal_connection::{send_line, SignalConnection};
use crate::share::signal_presence::build_presence;
use crate::share::types::{DirectAccessState, DirectContact, PeerPresence, ShareAuthState};
use crate::share::wire::ClientMsg;

const ACCESS_DOMAIN: &[u8] = b"se-server-access-v1\0";

/// `publish_direct`, `watch_direct` and `join_room` with the optional access
/// fields; without them the lines are the ones older servers know.
#[derive(Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum AccessClientMsg<'a> {
    PublishDirect {
        presence: &'a PeerPresence,
        #[serde(skip_serializing_if = "Option::is_none")]
        access_hash: Option<String>,
    },
    WatchDirect {
        lookup_id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        access_proof: Option<String>,
    },
    JoinRoom {
        room_id: &'a str,
        presence: &'a PeerPresence,
        #[serde(skip_serializing_if = "Option::is_none")]
        access_proof: Option<String>,
    },
}

/// HMAC-SHA256(secret, domain ‖ kind ‖ 0 ‖ relation id).
pub(in crate::share) fn access_proof(
    secret: &[u8],
    kind: &str,
    relation_id: &str,
) -> Option<[u8; 32]> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
    mac.update(ACCESS_DOMAIN);
    mac.update(kind.as_bytes());
    mac.update(&[0]);
    mac.update(relation_id.as_bytes());
    Some(mac.finalize().into_bytes().into())
}

/// What the owner leaves at the server: SHA-256 of the proof.
pub(in crate::share) fn access_hash(proof: &[u8; 32]) -> [u8; 32] {
    Sha256::digest(proof).into()
}

pub(in crate::share) fn publish_all(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    direct_requests_sent: &mut HashSet<String>,
    tracked_direct: bool,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let proofs = stream.key_login();
    if state.direct_online {
        let lookup_id = &state.identity.direct_lookup_id;
        let direct = build_presence(
            "direct",
            lookup_id,
            &state.identity,
            &state.direct_secret,
            iroh,
        )?;
        let access_hash = proofs
            .then(|| access_proof(&state.direct_secret, "direct", lookup_id))
            .flatten()
            .map(|proof| hex(&access_hash(&proof)));
        send_line(
            stream,
            &AccessClientMsg::PublishDirect {
                presence: &direct,
                access_hash,
            },
        )?;
    }
    for contact in state
        .direct_contacts
        .iter()
        .filter(|contact| contact.auto_connect)
    {
        let access_proof = proofs
            .then(|| {
                ShareProfiles::direct_secret_checked(contact)
                    .ok()
                    .flatten()
                    .and_then(|secret| access_proof(&secret, "direct", &contact.lookup_id))
            })
            .flatten()
            .map(|proof| hex(&proof));
        send_line(
            stream,
            &AccessClientMsg::WatchDirect {
                lookup_id: &contact.lookup_id,
                access_proof,
            },
        )?;
        if !tracked_direct
            && contact.access_state == DirectAccessState::Pending
            && !direct_requests_sent.contains(&contact.id)
        {
            send_direct_request_locked(stream, &state, contact, iroh)?;
            direct_requests_sent.insert(contact.id.clone());
        }
    }
    for room in state.rooms.iter().filter(|room| room.auto_join) {
        if let Some(secret) = ShareProfiles::room_secret_checked(room).map_err(eio)? {
            let presence = build_presence("room", &room.room_id, &state.identity, &secret, iroh)?;
            let access_proof = proofs
                .then(|| access_proof(&secret, "room", &room.room_id))
                .flatten()
                .map(|proof| hex(&proof));
            send_line(
                stream,
                &AccessClientMsg::JoinRoom {
                    room_id: &room.room_id,
                    presence: &presence,
                    access_proof,
                },
            )?;
        }
    }
    Ok(())
}

pub(in crate::share) fn send_direct_request(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    contact_id: &str,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let contact = state
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or_else(|| eio("Direktgeraet nicht gefunden"))?;
    send_direct_request_locked(stream, &state, contact, iroh)
}

fn send_direct_request_locked(
    stream: &mut SignalConnection,
    state: &ShareAuthState,
    contact: &DirectContact,
    iroh: &ShareIrohNode,
) -> io::Result<()> {
    let secret = ShareProfiles::direct_secret_checked(contact)
        .map_err(eio)?
        .ok_or_else(|| eio("Direkt-Secret fehlt"))?;
    let request = build_presence("direct", &contact.lookup_id, &state.identity, &secret, iroh)?;
    send_line(
        stream,
        &ClientMsg::RequestDirect {
            lookup_id: contact.lookup_id.clone(),
            presence: request,
        },
    )
}

pub(in crate::share) fn send_direct_answer(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    lookup_id: String,
    requester_device_id: String,
    accepted: bool,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let presence = Some(build_presence(
        "direct",
        &lookup_id,
        &state.identity,
        &state.direct_secret,
        iroh,
    )?);
    send_line(
        stream,
        &ClientMsg::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            msg: None,
        },
    )
}

#[cfg(test)]
mod review_task_tests {
    use super::{access_hash, access_proof, AccessClientMsg};
    use crate::share::types::PeerPresence;

    /// Shared test vectors with `se-share-server` (`access.rs`).
    #[test]
    fn review_task_access_proof_and_hash_match_the_server_vectors() {
        let proof = access_proof(&[7; 32], "direct", "lookup-a").expect("proof");
        assert_eq!(crate::share::core::hex(&proof), ACCESS_PROOF_VECTOR);
        assert_eq!(
            crate::share::core::hex(&access_hash(&proof)),
            ACCESS_HASH_VECTOR
        );
        assert_ne!(
            access_proof(&[7; 32], "room", "lookup-a"),
            Some(proof),
            "kinds are separated"
        );
    }

    #[test]
    fn review_task_access_fields_are_omitted_for_older_servers() {
        let watch = AccessClientMsg::WatchDirect {
            lookup_id: "lookup",
            access_proof: None,
        };
        assert_eq!(
            serde_json::to_string(&watch).unwrap(),
            r#"{"t":"watch_direct","lookup_id":"lookup"}"#
        );
        let presence = PeerPresence {
            kind: "room".into(),
            relation_id: "room".into(),
            device_id: "device".into(),
            device_name: "Device".into(),
            public_key: "pk".into(),
            fingerprint: "fp".into(),
            node_id: "node".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 1,
            nonce: "n".into(),
            proof: "p".into(),
        };
        let join = serde_json::to_value(AccessClientMsg::JoinRoom {
            room_id: "room",
            presence: &presence,
            access_proof: Some("ab".into()),
        })
        .unwrap();
        assert_eq!(join["t"], "join_room");
        assert_eq!(join["access_proof"], "ab");
        assert_eq!(join["presence"]["device_id"], "device");
    }

    const ACCESS_PROOF_VECTOR: &str =
        "b58b9881a79cc470538f046c183d9a1e34f56a1470faaea976c54e8098f85a49";
    const ACCESS_HASH_VECTOR: &str =
        "2d2db6518dc0bc5c8d0fe95916db6d07b1a7087d75fd90878af7841cc11653c6";
}
