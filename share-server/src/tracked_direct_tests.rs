use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use super::state::{Client, State};
use super::tracked_direct::{
    self, DirectDecisionKind, DirectPeerIdentity, DirectRoute, DirectRouteOutcome,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt, CAPABILITY,
};
use super::{In, Out, PeerPresence};

#[test]
fn tracked_request_routes_by_lookup_and_duplicates_remain_forwardable() {
    let (state, origin_rx, target_rx) = routed_state("requester", "target", Some("lookup"));
    let origin = state.lock().unwrap().clients[&1].writer.clone();
    let request = signed_request();

    tracked_direct::route_request(1, &origin, request.clone(), None, &state);
    tracked_direct::route_request(1, &origin, request.clone(), None, &state);

    for _ in 0..2 {
        match target_rx.recv().unwrap() {
            Out::DirectRequest { request: got } => assert_eq!(got, request),
            _ => panic!("wrong tracked request route"),
        }
        assert_ack(
            origin_rx.recv().unwrap(),
            DirectRoute::Request,
            DirectRouteOutcome::Forwarded,
        );
    }
}

#[test]
fn target_receipt_routes_to_requester_device() {
    let (state, requester_rx, target_rx) = routed_state("requester", "target", None);
    let target = state.lock().unwrap().clients[&2].writer.clone();
    let receipt = signed_request_receipt();

    tracked_direct::route_request_receipt(2, &target, receipt.clone(), &state);

    match requester_rx.recv().unwrap() {
        Out::DirectRequestReceipt { receipt: got } => assert_eq!(got, receipt),
        _ => panic!("wrong tracked target-receipt route"),
    }
    assert_ack(
        target_rx.recv().unwrap(),
        DirectRoute::RequestReceipt,
        DirectRouteOutcome::Forwarded,
    );
}

#[test]
fn signed_decision_routes_to_requester_device() {
    let (state, requester_rx, target_rx) = routed_state("requester", "target", None);
    let target = state.lock().unwrap().clients[&2].writer.clone();
    let decision = signed_decision();

    tracked_direct::route_decision(2, &target, decision.clone(), &state);

    match requester_rx.recv().unwrap() {
        Out::DirectDecision { decision: got } => assert_eq!(got, decision),
        _ => panic!("wrong tracked decision route"),
    }
    assert_ack(
        target_rx.recv().unwrap(),
        DirectRoute::Decision,
        DirectRouteOutcome::Forwarded,
    );
}

#[test]
fn requester_decision_receipt_routes_to_target_device() {
    let (state, requester_rx, target_rx) = routed_state("requester", "target", None);
    let requester = state.lock().unwrap().clients[&1].writer.clone();
    let receipt = signed_decision_receipt();

    tracked_direct::route_decision_receipt(1, &requester, receipt.clone(), &state);

    match target_rx.recv().unwrap() {
        Out::DirectDecisionReceipt { receipt: got } => assert_eq!(got, receipt),
        _ => panic!("wrong tracked decision-receipt route"),
    }
    assert_ack(
        requester_rx.recv().unwrap(),
        DirectRoute::DecisionReceipt,
        DirectRouteOutcome::Forwarded,
    );
}

#[test]
fn offline_request_gets_correlated_relay_ack_without_peer_receipt_claim() {
    let (origin_tx, origin_rx) = super::Writer::test_channel(8);
    let mut state = State::default();
    state
        .clients
        .insert(1, client(origin_tx.clone(), "requester", true));
    let state = Arc::new(Mutex::new(state));

    tracked_direct::route_request(1, &origin_tx, signed_request(), None, &state);

    assert_ack(
        origin_rx.recv().unwrap(),
        DirectRoute::Request,
        DirectRouteOutcome::TargetOffline,
    );
}

#[test]
fn oversized_tracked_envelopes_are_rejected_without_forward_or_ack() {
    // Rows: an oversized tracked envelope or legacy field from the requester (1) or the
    // target (2). The sender gets the error, the other device nothing; each row has its state.
    type Row = (
        &'static str,
        Option<&'static str>,
        u64,
        fn(&Arc<Mutex<State>>, &super::Writer),
    );
    let cases: [Row; 6] = [
        (
            "tracked request",
            Some("lookup"),
            1,
            |state: &Arc<Mutex<State>>, writer: &super::Writer| {
                let mut request = signed_request();
                request.request_id = "r".repeat(2 * 1024 * 1024);
                tracked_direct::route_request(1, writer, request, None, state);
            },
        ),
        (
            "tracked request receipt",
            None,
            2,
            |state: &Arc<Mutex<State>>, writer: &super::Writer| {
                let mut receipt = signed_request_receipt();
                receipt.request_digest = "d".repeat(257);
                tracked_direct::route_request_receipt(2, writer, receipt, state);
            },
        ),
        (
            "tracked decision",
            None,
            2,
            |state: &Arc<Mutex<State>>, writer: &super::Writer| {
                let mut decision = signed_decision();
                decision.message = Some("m".repeat(4097));
                tracked_direct::route_decision(2, writer, decision, state);
            },
        ),
        (
            "tracked decision receipt",
            None,
            1,
            |state: &Arc<Mutex<State>>, writer: &super::Writer| {
                let mut receipt = signed_decision_receipt();
                receipt.identity_signature = "s".repeat(1025);
                tracked_direct::route_decision_receipt(1, writer, receipt, state);
            },
        ),
        (
            "legacy request lookup",
            Some("lookup"),
            1,
            |state: &Arc<Mutex<State>>, writer: &super::Writer| {
                tracked_direct::request_legacy(
                    1,
                    writer,
                    &"l".repeat(257),
                    presence("requester"),
                    state,
                );
            },
        ),
        (
            "legacy decision message",
            Some("lookup"),
            2,
            |state: &Arc<Mutex<State>>, writer: &super::Writer| {
                tracked_direct::decision_legacy(
                    2,
                    writer,
                    "lookup",
                    "requester",
                    true,
                    Some(presence("target")),
                    Some("m".repeat(4097)),
                    state,
                );
            },
        ),
    ];
    for (case, lookup, sender_id, send) in cases {
        let (state, requester_rx, target_rx) = routed_state("requester", "target", lookup);
        let sender = state.lock().expect(case).clients[&sender_id].writer.clone();
        send(&state, &sender);
        let (sender_rx, other_rx) = if sender_id == 1 {
            (&requester_rx, &target_rx)
        } else {
            (&target_rx, &requester_rx)
        };
        assert_error(case, sender_rx.recv().expect(case));
        assert!(other_rx.try_recv().is_err(), "{case}");
    }
}

#[test]
fn tracked_wire_tags_and_ack_values_are_stable() {
    let request = signed_request();
    let request_receipt = signed_request_receipt();
    let decision = signed_decision();
    let decision_receipt = signed_decision_receipt();

    assert!(matches!(
        serde_json::from_value::<In>(serde_json::json!({
            "t": "submit_direct_request",
            "request": request,
        }))
        .unwrap(),
        In::SubmitDirectRequest { .. }
    ));
    assert!(matches!(
        serde_json::from_value::<In>(serde_json::json!({
            "t": "submit_direct_request_receipt",
            "receipt": request_receipt,
        }))
        .unwrap(),
        In::SubmitDirectRequestReceipt { .. }
    ));
    assert!(matches!(
        serde_json::from_value::<In>(serde_json::json!({
            "t": "submit_direct_decision",
            "decision": decision,
        }))
        .unwrap(),
        In::SubmitDirectDecision { .. }
    ));
    assert!(matches!(
        serde_json::from_value::<In>(serde_json::json!({
            "t": "submit_direct_decision_receipt",
            "receipt": decision_receipt,
        }))
        .unwrap(),
        In::SubmitDirectDecisionReceipt { .. }
    ));

    for (message, expected_tag) in [
        (
            Out::DirectRequest {
                request: signed_request(),
            },
            "direct_request",
        ),
        (
            Out::DirectRequestReceipt {
                receipt: signed_request_receipt(),
            },
            "direct_request_receipt",
        ),
        (
            Out::DirectDecision {
                decision: signed_decision(),
            },
            "direct_decision",
        ),
        (
            Out::DirectDecisionReceipt {
                receipt: signed_decision_receipt(),
            },
            "direct_decision_receipt",
        ),
    ] {
        assert_eq!(serde_json::to_value(message).unwrap()["t"], expected_tag);
    }

    let ack = serde_json::to_value(Out::DirectRouteAck {
        request_id: signed_request().request_id,
        route: DirectRoute::DecisionReceipt,
        outcome: DirectRouteOutcome::TargetOffline,
    })
    .unwrap();
    assert_eq!(ack["t"], "direct_route_ack");
    assert_eq!(ack["route"], "decision_receipt");
    assert_eq!(ack["outcome"], "target_offline");
}

fn assert_ack(message: Out, route: DirectRoute, outcome: DirectRouteOutcome) {
    match message {
        Out::DirectRouteAck {
            request_id,
            route: got_route,
            outcome: got_outcome,
        } => {
            assert_eq!(request_id, "550e8400-e29b-41d4-a716-446655440000");
            assert_eq!(got_route, route);
            assert_eq!(got_outcome, outcome);
        }
        _ => panic!("route did not return a correlated ACK"),
    }
}

fn assert_error(case: &str, message: Out) {
    match message {
        Out::Error { scope, msg } => {
            assert_eq!(scope, "direct", "{case}");
            assert!(msg.len() < 128, "{case}");
        }
        _ => panic!("{case}: invalid envelope was not rejected with a bounded error"),
    }
}

fn routed_state(
    requester_device: &str,
    target_device: &str,
    lookup: Option<&str>,
) -> (Arc<Mutex<State>>, Receiver<Out>, Receiver<Out>) {
    let (requester_tx, requester_rx) = super::Writer::test_channel(8);
    let (target_tx, target_rx) = super::Writer::test_channel(8);
    let mut state = State {
        clients: HashMap::from([
            (1, client(requester_tx, requester_device, true)),
            (2, client(target_tx, target_device, true)),
        ]),
        ..State::default()
    };
    // Receipts and decisions require the target's current publication too.
    state.direct.insert(
        lookup.unwrap_or("lookup").into(),
        (2, presence(target_device)),
    );
    (Arc::new(Mutex::new(state)), requester_rx, target_rx)
}

fn client(writer: super::Writer, device_id: &str, capable: bool) -> Client {
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
        identity: Default::default(),
    }
}

fn identity(device_id: &str) -> DirectPeerIdentity {
    DirectPeerIdentity {
        device_id: device_id.into(),
        device_name: format!("Device {device_id}"),
        node_id: format!("node-{device_id}"),
        public_key: format!("key-{device_id}"),
        fingerprint: format!("fingerprint-{device_id}"),
    }
}

fn signed_request() -> SignedDirectRequest {
    SignedDirectRequest {
        request_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        lookup_id: "lookup".into(),
        requester: identity("requester"),
        target: identity("target"),
        created_at: 10,
        expires_at: 20,
        nonce: "request-nonce".into(),
        message: Some("please allow".into()),
        hmac_proof: "request-hmac".into(),
        identity_signature: "request-signature".into(),
    }
}

fn signed_request_receipt() -> SignedDirectRequestReceipt {
    SignedDirectRequestReceipt {
        request_id: signed_request().request_id,
        lookup_id: "lookup".into(),
        requester: identity("requester"),
        target: identity("target"),
        request_digest: "request-digest".into(),
        received_at: 11,
        expires_at: 21,
        nonce: "receipt-nonce".into(),
        message: None,
        hmac_proof: "receipt-hmac".into(),
        identity_signature: "receipt-signature".into(),
    }
}

fn signed_decision() -> SignedDirectDecision {
    SignedDirectDecision {
        request_id: signed_request().request_id,
        lookup_id: "lookup".into(),
        requester: identity("requester"),
        target: identity("target"),
        request_digest: "request-digest".into(),
        decision: DirectDecisionKind::Accepted,
        decision_revision: 1,
        decided_at: 12,
        expires_at: 22,
        nonce: "decision-nonce".into(),
        message: Some("accepted".into()),
        hmac_proof: "decision-hmac".into(),
        identity_signature: "decision-signature".into(),
    }
}

fn signed_decision_receipt() -> SignedDirectDecisionReceipt {
    SignedDirectDecisionReceipt {
        request_id: signed_request().request_id,
        lookup_id: "lookup".into(),
        requester: identity("requester"),
        target: identity("target"),
        decision_digest: "decision-digest".into(),
        decision: DirectDecisionKind::Accepted,
        decision_revision: 1,
        received_at: 13,
        expires_at: 23,
        nonce: "decision-receipt-nonce".into(),
        message: None,
        hmac_proof: "decision-receipt-hmac".into(),
        identity_signature: "decision-receipt-signature".into(),
    }
}

fn presence(device_id: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: device_id.into(),
        device_name: device_id.into(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        relay_url: "relay".into(),
        candidates: Vec::new(),
        expires_at: 99,
        nonce: "n".into(),
        proof: "proof".into(),
    }
}
