//! r-entdeckung-1 (suite-2-entdeckung), end to end through the verifying
//! paths: a flood of signed presences no longer blocks a pinned contact, and
//! the downgrade protection of signing contacts and of recent markers holds.
use std::sync::{Arc, Mutex};

use super::*;
use crate::share::power::test_support::{auth_state, identity};
use crate::share::signal_auth::{verify_direct_access_accepted_using, verify_local_direct_request};
use crate::share::types::{DirectAccessState, DirectContact, ShareAuthState, ShareStatus};

/// More signed presences than the former shared bound of 4096 entries could
/// hold (each one left a replay key and a marker).
const FLOOD: usize = 2_100;
const CONTACT_LOOKUP: &str = "entdeckung-contact-lookup";

fn signer(index: usize) -> ShareIdentity {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&(index as u64 + 1).to_be_bytes());
    bytes[31] = 0x5a;
    let secret = iroh::SecretKey::from_bytes(&bytes);
    let node_id = secret.public().to_string();
    ShareIdentity {
        device_id: format!("flood-device-{index}"),
        device_name: format!("Flood {index}"),
        direct_lookup_id: format!("flood-lookup-{index}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: [3; 32],
    }
}

fn signed(
    identity: &ShareIdentity,
    relation_id: &str,
    secret: &[u8],
    nonce_random: &str,
) -> PeerPresence {
    sign_presence(
        "direct",
        relation_id,
        identity,
        secret,
        String::new(),
        Vec::new(),
        nonce_random.to_string(),
    )
}

/// The same presence without the device signature, with a valid relation proof.
fn unsigned_variant(presence: &PeerPresence, secret: &[u8], nonce: &str) -> PeerPresence {
    let mut plain = presence.clone();
    plain.nonce = nonce.to_string();
    plain.proof = hmac_proof(
        secret,
        &presence_payload(
            "direct",
            &plain.relation_id,
            &plain.device_id,
            &plain.public_key,
            &plain.node_id,
            &plain.relay_url,
            &plain.candidates,
            plain.expires_at,
            &plain.nonce,
        ),
    );
    plain
}

fn contact_for(peer: &ShareIdentity, signed_presence: bool) -> DirectContact {
    let mut contact = DirectContact {
        id: "contact-entdeckung".into(),
        display_name: "Contact".into(),
        lookup_id: CONTACT_LOOKUP.into(),
        expected_fingerprint: peer.fingerprint.clone(),
        expected_node_id: peer.node_id.clone(),
        remote_device_id: Some(peer.device_id.clone()),
        remote_public_key: Some(peer.public_key.clone()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(peer.public_key.clone()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    };
    contact.relation.signed_presence = signed_presence;
    contact
}

fn local_state() -> (ShareIdentity, Arc<Mutex<ShareAuthState>>, Vec<u8>) {
    let local = identity("flood local").expect("identity");
    let state = auth_state(&local);
    let secret = state.direct_secret.clone();
    (local, Arc::new(Mutex::new(state)), secret)
}

#[test]
fn entdeckung_task_a_flood_of_signed_requests_does_not_block_a_pinned_contact() {
    let (local, auth, secret) = local_state();
    for index in 0..FLOOD {
        let request = signed(
            &signer(index),
            &local.direct_lookup_id,
            &secret,
            &format!("rp1.flood-{index}"),
        );
        assert!(
            verify_local_direct_request(&local.direct_lookup_id, &request, &auth),
            "signed request {index} is verified"
        );
    }
    {
        let state = auth.lock().expect("auth");
        let markers = state
            .seen_nonces
            .iter()
            .filter(|entry| entry.starts_with("signed|"))
            .count();
        assert!(markers <= 1_024, "{markers} markers");
    }

    // A pinned contact still gets through after the flood.
    let contact_secret = vec![11u8; 32];
    let peer = signer(10_000);
    auth.lock()
        .expect("auth")
        .direct_contacts
        .push(contact_for(&peer, false));
    let presence = signed(&peer, CONTACT_LOOKUP, &contact_secret, "rp1.pinned");
    assert!(verify_direct_access_accepted_using(
        CONTACT_LOOKUP,
        &local.device_id,
        Some(&presence),
        &auth,
        |_| Some(contact_secret.clone()),
    ));
    assert!(
        auth.lock().expect("auth").direct_contacts[0]
            .relation
            .signed_presence
    );

    // The marker of a recent signer still blocks its downgrade to unsigned;
    // a key that never signed stays accepted (older devices).
    let recent = signed(
        &signer(FLOOD - 1),
        &local.direct_lookup_id,
        &secret,
        "rp1.recent",
    );
    let downgrade = unsigned_variant(&recent, &secret, "plain-recent");
    assert!(!verify_local_direct_request(
        &local.direct_lookup_id,
        &downgrade,
        &auth
    ));
    let never_signed = signed(
        &signer(900_000),
        &local.direct_lookup_id,
        &secret,
        "rp1.never",
    );
    let legacy = unsigned_variant(&never_signed, &secret, "plain-legacy");
    assert!(verify_local_direct_request(
        &local.direct_lookup_id,
        &legacy,
        &auth
    ));
}

#[test]
fn entdeckung_task_an_unsigned_presence_of_a_signing_contact_is_refused() {
    let peer = signer(20_000);
    let contact_secret = vec![12u8; 32];
    let signed_presence = signed(&peer, CONTACT_LOOKUP, &contact_secret, "rp1.signed");
    let plain = unsigned_variant(&signed_presence, &contact_secret, "plain-nonce-1");

    // The contact delivered a signed presence before: no downgrade.
    let (local, auth, _) = local_state();
    auth.lock()
        .expect("auth")
        .direct_contacts
        .push(contact_for(&peer, true));
    assert!(!verify_direct_access_accepted_using(
        CONTACT_LOOKUP,
        &local.device_id,
        Some(&plain),
        &auth,
        |_| Some(contact_secret.clone()),
    ));

    // The same presence of a contact that never signed is still accepted.
    let (local, auth, _) = local_state();
    auth.lock()
        .expect("auth")
        .direct_contacts
        .push(contact_for(&peer, false));
    assert!(verify_direct_access_accepted_using(
        CONTACT_LOOKUP,
        &local.device_id,
        Some(&plain),
        &auth,
        |_| Some(contact_secret.clone()),
    ));
}
