use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::state::{Client, State};
use super::tracked_direct::{
    self, DirectPeerIdentity, DirectRoute, DirectRouteOutcome, SignedDirectRequest, CAPABILITY,
};
use super::{Out, PeerPresence, Writer};

#[test]
fn tracked_request_is_bridged_to_online_legacy_target() {
    let (state, origin_rx, target_rx) = state_with_legacy_target();
    let origin = state.lock().unwrap().clients[&1].writer.clone();
    let request = request();

    tracked_direct::route_request(
        1,
        &origin,
        request.clone(),
        Some(legacy_presence(&request)),
        &state,
    );

    assert!(matches!(
        target_rx.recv().unwrap(),
        Out::DirectAccessRequest { lookup_id, presence }
            if lookup_id == request.lookup_id && presence.device_id == request.requester.device_id
    ));
    assert!(matches!(
        origin_rx.recv().unwrap(),
        Out::DirectRouteAck {
            request_id,
            route: DirectRoute::Request,
            outcome: DirectRouteOutcome::LegacyForwarded,
        } if request_id == request.request_id
    ));
}

#[test]
fn legacy_bridge_identity_mismatch_is_rejected_without_forwarding() {
    let (state, origin_rx, target_rx) = state_with_legacy_target();
    let origin = state.lock().unwrap().clients[&1].writer.clone();
    let request = request();
    let mut presence = legacy_presence(&request);
    presence.node_id = "other-node".into();

    tracked_direct::route_request(1, &origin, request, Some(presence), &state);

    assert!(matches!(origin_rx.recv().unwrap(), Out::Error { .. }));
    assert!(target_rx.try_recv().is_err());
}

fn state_with_legacy_target() -> (
    Arc<Mutex<State>>,
    std::sync::mpsc::Receiver<Out>,
    std::sync::mpsc::Receiver<Out>,
) {
    let (origin, origin_rx) = Writer::test_channel(8);
    let (target, target_rx) = Writer::test_channel(8);
    let mut state = State {
        clients: HashMap::from([
            (1, client(origin, "requester", true)),
            (2, client(target, "target", false)),
        ]),
        ..State::default()
    };
    state
        .direct
        .insert("lookup".into(), (2, presence("target", "lookup")));
    (Arc::new(Mutex::new(state)), origin_rx, target_rx)
}

fn client(writer: Writer, device_id: &str, capable: bool) -> Client {
    Client {
        writer,
        source: super::limits::SourceKey::Ipv4([127, 0, 0, 1]),
        device_id: device_id.into(),
        capabilities: if capable {
            HashSet::from([CAPABILITY.to_string()])
        } else {
            HashSet::new()
        },
        direct_lookup_ids: HashSet::new(),
        watched_lookup_ids: HashSet::new(),
        rooms: HashSet::new(),
    }
}

fn request() -> SignedDirectRequest {
    SignedDirectRequest {
        request_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        lookup_id: "lookup".into(),
        requester: DirectPeerIdentity {
            device_id: "requester".into(),
            device_name: "Requester".into(),
            node_id: "requester-node".into(),
            public_key: "requester-key".into(),
            fingerprint: "requester-fingerprint".into(),
        },
        target: DirectPeerIdentity {
            device_id: "target".into(),
            device_name: "Target".into(),
            node_id: "target-node".into(),
            public_key: "target-key".into(),
            fingerprint: "target-fingerprint".into(),
        },
        created_at: 10,
        expires_at: 20,
        nonce: "request-nonce".into(),
        message: None,
        hmac_proof: "request-hmac".into(),
        identity_signature: "request-signature".into(),
    }
}

fn legacy_presence(request: &SignedDirectRequest) -> PeerPresence {
    let requester = &request.requester;
    PeerPresence {
        kind: "direct".into(),
        relation_id: request.lookup_id.clone(),
        device_id: requester.device_id.clone(),
        device_name: requester.device_name.clone(),
        public_key: requester.public_key.clone(),
        fingerprint: requester.fingerprint.clone(),
        node_id: requester.node_id.clone(),
        relay_url: "http://127.0.0.1:51821".into(),
        candidates: vec!["127.0.0.1:1".into()],
        expires_at: 99,
        nonce: "legacy-nonce".into(),
        proof: "legacy-proof".into(),
    }
}

fn presence(device_id: &str, relation_id: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: relation_id.into(),
        device_id: device_id.into(),
        device_name: device_id.into(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        relay_url: "relay".into(),
        candidates: Vec::new(),
        expires_at: 99,
        nonce: "nonce".into(),
        proof: "proof".into(),
    }
}
