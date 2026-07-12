use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::tracked_direct::{
    self, DirectDecisionKind, DirectPeerIdentity, DirectRoute, DirectRouteOutcome,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt, CAPABILITY,
};
use super::{handle, Client, In, Out, PeerPresence, State};

#[test]
fn tracked_request_routes_by_lookup_and_duplicates_remain_forwardable() {
    let (state, origin_rx, target_rx) = routed_state("requester", "target", Some("lookup"));
    let origin = state.lock().unwrap().clients[&1].writer.clone();
    let request = signed_request();

    tracked_direct::route_request(1, &origin, request.clone(), &state);
    tracked_direct::route_request(1, &origin, request.clone(), &state);

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
    let (origin_tx, origin_rx) = mpsc::channel();
    let mut state = State::default();
    state
        .clients
        .insert(1, client(origin_tx.clone(), "requester", true));
    let state = Arc::new(Mutex::new(state));

    tracked_direct::route_request(1, &origin_tx, signed_request(), &state);

    assert_ack(
        origin_rx.recv().unwrap(),
        DirectRoute::Request,
        DirectRouteOutcome::TargetOffline,
    );
}

#[test]
fn capable_tcp_hello_negotiates_only_tracked_direct_v1() {
    let response = tcp_hello(Some(&[CAPABILITY, "unsupported"]));
    assert_eq!(response["t"], "hello_ok");
    assert_eq!(response["capabilities"], serde_json::json!([CAPABILITY]));
}

#[test]
fn legacy_tcp_hello_response_remains_byte_compatible() {
    let response = tcp_hello(None);
    assert_eq!(response, serde_json::json!({"t": "hello_ok"}));
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

fn routed_state(
    requester_device: &str,
    target_device: &str,
    lookup: Option<&str>,
) -> (Arc<Mutex<State>>, Receiver<Out>, Receiver<Out>) {
    let (requester_tx, requester_rx) = mpsc::channel();
    let (target_tx, target_rx) = mpsc::channel();
    let mut state = State {
        clients: HashMap::from([
            (1, client(requester_tx, requester_device, true)),
            (2, client(target_tx, target_device, true)),
        ]),
        ..State::default()
    };
    if let Some(lookup) = lookup {
        state
            .direct
            .insert(lookup.into(), (2, presence(target_device)));
    }
    (Arc::new(Mutex::new(state)), requester_rx, target_rx)
}

fn client(writer: super::Writer, device_id: &str, capable: bool) -> Client {
    Client {
        writer,
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

fn tcp_hello(capabilities: Option<&[&str]>) -> serde_json::Value {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle(stream, Arc::new(Mutex::new(State::default()))).unwrap();
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut hello = serde_json::json!({
        "t": "hello",
        "protocol_version": 3,
        "device_id": "requester",
        "device_name": "Requester",
        "listen_port": 0,
        "lan": [],
        "public_key": "pk",
        "fingerprint": "fp"
    });
    if let Some(capabilities) = capabilities {
        hello["capabilities"] = serde_json::json!(capabilities);
    }
    writeln!(stream, "{hello}").unwrap();
    stream.flush().unwrap();

    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    stream.shutdown(Shutdown::Both).unwrap();
    server.join().unwrap();
    serde_json::from_str(line.trim()).unwrap()
}
