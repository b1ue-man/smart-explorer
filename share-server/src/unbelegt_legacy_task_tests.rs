//! Clients without key login in compatibility mode (E60, S15): such a client publishes
//! no Direct presence, watches only unbound lookups and reaches a bound lookup on no
//! request path (legacy request, legacy decision, signed request with and without the
//! legacy bridge, signed decision receipt). A proven client publishes and requests as
//! before, a watcher with the matching proof still receives the presence, and a
//! request to an unbound lookup is still routed (mixed operation).

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh_base::SecretKey;

use crate::direct_presence;
use crate::limits::SourceKey;
use crate::login;
use crate::state::{
    register_client, register_client_with_identity, watcher_admitted, ClientIdentity,
    RegistrationError, State,
};
use crate::tracked_direct;
use crate::{Out, PeerPresence, Writer};

const WAIT: Duration = Duration::from_secs(2);
/// How long a test waits to see that no message arrives.
const QUIET: Duration = Duration::from_millis(200);
const LOOKUP: &str = "lookup";

/// A client registered without key login (`proven` is false) or with its proven key.
struct Peer {
    id: u64,
    device: String,
    key: SecretKey,
    writer: Writer,
    receiver: Receiver<Out>,
}

fn peer(state: &Arc<Mutex<State>>, device: &str, seed: u8, proven: bool) -> Peer {
    peer_with(state, device, seed, proven, HashSet::new())
}

/// A client that negotiated `tracked_direct_v1` (the signed request paths).
fn capable_peer(state: &Arc<Mutex<State>>, device: &str, seed: u8, proven: bool) -> Peer {
    let capabilities = HashSet::from([tracked_direct::CAPABILITY.to_string()]);
    peer_with(state, device, seed, proven, capabilities)
}

fn peer_with(
    state: &Arc<Mutex<State>>,
    device: &str,
    seed: u8,
    proven: bool,
    capabilities: HashSet<String>,
) -> Peer {
    let key = SecretKey::from_bytes(&[seed; 32]);
    let (writer, receiver) = Writer::test_channel(32);
    let source = SourceKey::Ipv4([192, 0, 2, seed]);
    let registration = if proven {
        register_client_with_identity(
            state,
            writer.clone(),
            source,
            device.into(),
            capabilities,
            ClientIdentity::Proven(key.public()),
        )
    } else {
        register_client(state, writer.clone(), source, device.into(), capabilities)
    };
    Peer {
        id: registration.expect("registration"),
        device: device.into(),
        key,
        writer,
        receiver,
    }
}

fn presence(peer: &Peer, lookup: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: lookup.into(),
        device_id: peer.device.clone(),
        device_name: peer.device.clone(),
        public_key: peer.key.public().to_string(),
        fingerprint: "fp".into(),
        node_id: peer.key.public().to_string(),
        relay_url: "https://relay.example".into(),
        candidates: Vec::new(),
        expires_at: crate::discovery_state::unix_seconds() + 300,
        nonce: "nonce".into(),
        proof: "proof".into(),
    }
}

/// Whether a binding to a key exists for `LOOKUP`.
fn is_bound(state: &Arc<Mutex<State>>) -> bool {
    state
        .lock()
        .expect("state")
        .bindings
        .lookup_key(LOOKUP)
        .is_some()
}

/// The client receives exactly the existing text of a missing key login, in scope `direct`.
fn assert_key_login_refusal(peer: &Peer) {
    match peer.receiver.recv_timeout(WAIT) {
        Ok(Out::Error { scope, msg }) => {
            assert_eq!(scope, "direct");
            assert_eq!(msg, RegistrationError::KeyLoginRequired.message());
        }
        _ => panic!("the key-login refusal was not sent"),
    }
}

#[test]
fn unbelegt_task_legacy_publish_unbound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let legacy = peer(&state, "old-phone", 71, false);

    direct_presence::publish(
        legacy.id,
        &legacy.writer,
        presence(&legacy, LOOKUP),
        None,
        &state,
    );

    // The refusal comes with the existing key-login text; nothing is recorded or bound.
    assert_key_login_refusal(&legacy);
    let guard = state.lock().expect("state");
    assert!(!guard.direct.contains_key(LOOKUP));
    assert!(guard.bindings.lookup_key(LOOKUP).is_none());
    assert!(guard.clients[&legacy.id].direct_lookup_ids.is_empty());
}

#[test]
fn unbelegt_task_legacy_watch_bound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let owner = peer(&state, "owner", 72, true);
    let legacy = peer(&state, "old-phone", 73, false);
    let hash = login::hex(&login::sha256(&[5; 32]));
    direct_presence::publish(
        owner.id,
        &owner.writer,
        presence(&owner, LOOKUP),
        Some(hash),
        &state,
    );
    assert!(is_bound(&state));

    direct_presence::watch(legacy.id, &legacy.writer, LOOKUP, None, &state);

    // The bound lookup is refused, and the watcher is not kept for it.
    assert_key_login_refusal(&legacy);
    {
        let guard = state.lock().expect("state");
        assert!(!guard
            .watchers
            .get(LOOKUP)
            .is_some_and(|watchers| watchers.contains(&legacy.id)));
        let client = &guard.clients[&legacy.id];
        assert!(!client.watched_lookup_ids.contains(LOOKUP));
        assert!(!watcher_admitted(&guard, legacy.id, LOOKUP));
    }
    // The owner leaving later does not reach the refused client either.
    direct_presence::unpublish(owner.id, LOOKUP, &state);
    assert!(legacy.receiver.recv_timeout(QUIET).is_err());
}

#[test]
fn unbelegt_task_legacy_watch_unbound_lookup_stays_admitted_until_bound() {
    let state = Arc::new(Mutex::new(State::default()));
    let owner = peer(&state, "owner", 74, true);
    let legacy = peer(&state, "old-phone", 75, false);
    // A publication of a proven client always binds its lookup (E60), so this entry is
    // written directly to model an unbound lookup that has a presence.
    state
        .lock()
        .expect("state")
        .direct
        .insert(LOOKUP.into(), (owner.id, presence(&owner, LOOKUP)));

    direct_presence::watch(legacy.id, &legacy.writer, LOOKUP, None, &state);

    // Admitted: no refusal, the watcher is kept, and the presence of the unbound lookup arrives.
    assert!(matches!(
        legacy.receiver.recv_timeout(WAIT),
        Ok(Out::DirectAvailable { .. })
    ));
    assert!(state.lock().expect("state").watchers[LOOKUP].contains(&legacy.id));

    // The owner's publication binds the lookup, so the watcher is not notified of it.
    let hash = login::hex(&login::sha256(&[5; 32]));
    direct_presence::publish(
        owner.id,
        &owner.writer,
        presence(&owner, LOOKUP),
        Some(hash),
        &state,
    );
    assert!(is_bound(&state));
    assert!(legacy.receiver.recv_timeout(QUIET).is_err());
}

#[test]
fn unbelegt_task_proven_publish_unchanged() {
    let state = Arc::new(Mutex::new(State::default()));
    let owner = peer(&state, "owner", 76, true);
    let watcher = peer(&state, "watcher", 77, true);
    let proof = login::hex(&[5; 32]);
    let hash = login::hex(&login::sha256(&[5; 32]));
    direct_presence::watch(watcher.id, &watcher.writer, LOOKUP, Some(proof), &state);

    direct_presence::publish(
        owner.id,
        &owner.writer,
        presence(&owner, LOOKUP),
        Some(hash),
        &state,
    );

    // The owner is not refused. The entry, the binding to the owner's key and the hash
    // are recorded as before, and the watcher with the matching proof is notified.
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
    assert!(matches!(
        watcher.receiver.recv_timeout(WAIT),
        Ok(Out::DirectAvailable { .. })
    ));
    let owner_key = owner.key.public().to_string();
    let guard = state.lock().expect("state");
    assert_eq!(guard.direct[LOOKUP].0, owner.id);
    assert_eq!(guard.bindings.lookup_key(LOOKUP), Some(owner_key.as_str()));
    assert_eq!(
        guard.bindings.lookup_access(LOOKUP),
        Some(login::sha256(&[5; 32]))
    );
    assert!(guard.clients[&owner.id].direct_lookup_ids.contains(LOOKUP));
}

/// A proven owner whose publication bound `LOOKUP`, and a client without key login.
fn bound_owner_and_legacy(state: &Arc<Mutex<State>>, seeds: (u8, u8)) -> (Peer, Peer) {
    let owner = peer(state, "owner", seeds.0, true);
    let legacy = peer(state, "old-phone", seeds.1, false);
    let hash = login::hex(&login::sha256(&[5; 32]));
    direct_presence::publish(
        owner.id,
        &owner.writer,
        presence(&owner, LOOKUP),
        Some(hash),
        state,
    );
    assert!(is_bound(state));
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
    (owner, legacy)
}

#[test]
fn unbelegt_task_legacy_request_to_bound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let (owner, legacy) = bound_owner_and_legacy(&state, (78, 79));

    tracked_direct::request_legacy(
        legacy.id,
        &legacy.writer,
        LOOKUP,
        presence(&legacy, LOOKUP),
        &state,
    );

    // The requester gets the key-login text; its presence never reaches the owner.
    assert_key_login_refusal(&legacy);
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
}

#[test]
fn unbelegt_task_legacy_decision_to_bound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let (owner, legacy) = bound_owner_and_legacy(&state, (80, 81));

    tracked_direct::decision_legacy(
        legacy.id,
        &legacy.writer,
        LOOKUP,
        &owner.device,
        true,
        Some(presence(&legacy, LOOKUP)),
        None,
        &state,
    );

    // A decision for the bound lookup is refused; the named device receives nothing.
    assert_key_login_refusal(&legacy);
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
}

#[test]
fn unbelegt_task_legacy_request_to_unbound_lookup_still_routed() {
    let state = Arc::new(Mutex::new(State::default()));
    let owner = peer(&state, "owner", 82, true);
    let legacy = peer(&state, "old-phone", 83, false);
    // An unbound lookup with a presence, written directly as in the watch test above.
    state
        .lock()
        .expect("state")
        .direct
        .insert(LOOKUP.into(), (owner.id, presence(&owner, LOOKUP)));
    assert!(!is_bound(&state));

    tracked_direct::request_legacy(
        legacy.id,
        &legacy.writer,
        LOOKUP,
        presence(&legacy, LOOKUP),
        &state,
    );

    // Mixed operation: the request reaches the owner, the requester gets no refusal.
    match owner.receiver.recv_timeout(WAIT) {
        Ok(Out::DirectAccessRequest {
            lookup_id,
            presence,
        }) => {
            assert_eq!(lookup_id, LOOKUP);
            assert_eq!(presence.device_id, "old-phone");
        }
        _ => panic!("the request to the unbound lookup was not routed"),
    }
    assert!(legacy.receiver.recv_timeout(QUIET).is_err());
}

/// The identity a signed envelope names for `peer` (key login compares key and node).
fn signed_identity(peer: &Peer) -> tracked_direct::DirectPeerIdentity {
    tracked_direct::DirectPeerIdentity {
        device_id: peer.device.clone(),
        device_name: peer.device.clone(),
        node_id: peer.key.public().to_string(),
        public_key: peer.key.public().to_string(),
        fingerprint: "fp".into(),
    }
}

fn signed_request(requester: &Peer, owner: &Peer) -> tracked_direct::SignedDirectRequest {
    tracked_direct::SignedDirectRequest {
        request_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        lookup_id: LOOKUP.into(),
        requester: signed_identity(requester),
        target: signed_identity(owner),
        created_at: 10,
        expires_at: 20,
        nonce: "request-nonce".into(),
        message: None,
        hmac_proof: "request-hmac".into(),
        identity_signature: "request-signature".into(),
    }
}

/// A proven owner with `tracked_direct_v1` whose publication bound `LOOKUP`, and a
/// client without key login that negotiated the same capability.
fn bound_capable_owner_and_legacy(state: &Arc<Mutex<State>>, seeds: (u8, u8)) -> (Peer, Peer) {
    let owner = capable_peer(state, "owner", seeds.0, true);
    let legacy = capable_peer(state, "old-phone", seeds.1, false);
    let hash = login::hex(&login::sha256(&[5; 32]));
    direct_presence::publish(
        owner.id,
        &owner.writer,
        presence(&owner, LOOKUP),
        Some(hash),
        state,
    );
    assert!(is_bound(state));
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
    (owner, legacy)
}

#[test]
fn unbelegt_task_legacy_bridge_request_to_bound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let (owner, legacy) = bound_capable_owner_and_legacy(&state, (84, 85));

    // The legacy bridge of a signed request: `In::SubmitDirectRequest` with the
    // requester's presence for an owner without the capability.
    tracked_direct::route_request(
        legacy.id,
        &legacy.writer,
        signed_request(&legacy, &owner),
        Some(presence(&legacy, LOOKUP)),
        &state,
    );

    // Neither the request nor the presence reaches the owner, and no route
    // acknowledgement follows the refusal.
    assert_key_login_refusal(&legacy);
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
    assert!(legacy.receiver.recv_timeout(QUIET).is_err());
}

#[test]
fn unbelegt_task_legacy_signed_request_to_bound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let (owner, legacy) = bound_capable_owner_and_legacy(&state, (86, 87));

    // The signed request alone, without the bridge.
    tracked_direct::route_request(
        legacy.id,
        &legacy.writer,
        signed_request(&legacy, &owner),
        None,
        &state,
    );
    assert_key_login_refusal(&legacy);
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
    assert!(legacy.receiver.recv_timeout(QUIET).is_err());

    // A proven requester still reaches the owner of the bound lookup.
    let proven = capable_peer(&state, "new-phone", 88, true);
    let request = signed_request(&proven, &owner);
    tracked_direct::route_request(proven.id, &proven.writer, request.clone(), None, &state);
    match owner.receiver.recv_timeout(WAIT) {
        Ok(Out::DirectRequest { request: routed }) => assert_eq!(routed, request),
        _ => panic!("the signed request of a proven client was not routed"),
    }
    assert!(matches!(
        proven.receiver.recv_timeout(WAIT),
        Ok(Out::DirectRouteAck { .. })
    ));
}

#[test]
fn unbelegt_task_legacy_decision_receipt_for_bound_lookup_refused() {
    let state = Arc::new(Mutex::new(State::default()));
    let (owner, legacy) = bound_capable_owner_and_legacy(&state, (89, 90));

    // A decision receipt is routed by the target's device id: without the
    // refusal it would reach the owner's capable device.
    let receipt = tracked_direct::SignedDirectDecisionReceipt {
        request_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        lookup_id: LOOKUP.into(),
        requester: signed_identity(&legacy),
        target: signed_identity(&owner),
        decision_digest: "decision-digest".into(),
        decision: tracked_direct::DirectDecisionKind::Accepted,
        decision_revision: 1,
        received_at: 13,
        expires_at: 23,
        nonce: "decision-receipt-nonce".into(),
        message: None,
        hmac_proof: "decision-receipt-hmac".into(),
        identity_signature: "decision-receipt-signature".into(),
    };
    tracked_direct::route_decision_receipt(legacy.id, &legacy.writer, receipt, &state);

    assert_key_login_refusal(&legacy);
    assert!(owner.receiver.recv_timeout(QUIET).is_err());
    assert!(legacy.receiver.recv_timeout(QUIET).is_err());
}
