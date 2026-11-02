use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use crossbeam_channel::unbounded;

use super::core::public_fingerprint;
use super::direct_ledger::{DirectRequestDirection, DirectRequestEntry, DirectRequestRetries};
use super::direct_lifecycle::DirectRequestRecord;
use super::direct_protocol::{DirectPeerIdentity, DirectRequestId, SignedDirectRequest};
use super::fs::ShareExportConfig;
use super::identity::ShareIdentity;
use super::signal_connection::SignalConnection;
use super::tracked_signal_sender::send_pending_tracked_with;
use super::types::{PeerPresence, ShareAuthState, ShareEvent};

#[test]
fn missing_legacy_bridge_does_not_block_signed_request_or_later_outboxes() {
    let first = request(1, "01234567-89ab-4def-8123-456789abcdef");
    let second = request(2, "11234567-89ab-4def-8123-456789abcdef");
    let auth = Arc::new(Mutex::new(state(vec![
        entry(first.clone()),
        entry(second.clone()),
    ])));
    let (mut signal, peer) = connection_pair();
    let (events, event_rx) = unbounded();
    let mut counters = HashMap::new();

    let sent =
        send_pending_tracked_with(&mut signal, &auth, &events, &mut counters, |_, request| {
            if request.request_id == first.request_id {
                Err("missing relation secret".into())
            } else {
                Ok(presence(request))
            }
        })
        .unwrap();

    assert_eq!(sent, 2);
    let mut reader = BufReader::new(peer);
    let first_wire = read_json(&mut reader);
    let second_wire = read_json(&mut reader);
    assert_eq!(first_wire["t"], "submit_direct_request");
    assert_eq!(
        first_wire["request"]["request_id"],
        first.request_id.as_str()
    );
    assert!(first_wire.get("legacy_presence").is_none());
    assert_eq!(
        second_wire["legacy_presence"]["device_id"],
        second.requester.device_id
    );
    assert!(
        matches!(event_rx.recv().unwrap(), ShareEvent::Error(message) if message.contains("trotzdem gesendet"))
    );
    assert!(matches!(
        event_rx.recv().unwrap(),
        ShareEvent::DirectSignal(_)
    ));
    assert!(matches!(
        event_rx.recv().unwrap(),
        ShareEvent::DirectSignal(_)
    ));
}

fn state(direct_requests: Vec<DirectRequestEntry>) -> ShareAuthState {
    let secret = iroh::SecretKey::from_bytes(&[9; 32]);
    let public_key = secret.public().to_string();
    ShareAuthState {
        identity: ShareIdentity {
            device_id: "requester".into(),
            device_name: "Requester".into(),
            direct_lookup_id: "local-lookup".into(),
            fingerprint: public_fingerprint(public_key.as_bytes()),
            node_id: public_key.clone(),
            public_key,
            iroh_secret: secret,
            direct_secret: [4; 32],
        },
        direct_secret: vec![4; 32],
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests,
        direct_request_tombstones: Vec::new(),
        seen_nonces: HashSet::new(),
        direct_online: true,
        authorization_epoch: 0,
    }
}

fn entry(request: SignedDirectRequest) -> DirectRequestEntry {
    DirectRequestEntry {
        direction: DirectRequestDirection::Outgoing,
        contact_id: Some("missing-contact".into()),
        local_lookup_id: None,
        record: DirectRequestRecord::new(request),
        request_receipt: None,
        decision: None,
        decision_receipt: None,
        retries: DirectRequestRetries::default(),
    }
}

fn request(byte: u8, request_id: &str) -> SignedDirectRequest {
    let now = super::core::now_secs();
    let requester_secret = iroh::SecretKey::from_bytes(&[9; 32]);
    let target_secret = iroh::SecretKey::from_bytes(&[byte; 32]);
    let target =
        DirectPeerIdentity::from_secret(format!("target-{byte}"), "Target", &target_secret);
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse(request_id).unwrap(),
        format!("lookup-{byte}"),
        DirectPeerIdentity::from_secret("requester", "Requester", &requester_secret),
        DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
        now,
        now + 1_000,
        format!("nonce-{byte}"),
        None,
        &[byte; 32],
        &requester_secret,
    )
    .unwrap()
}

fn presence(request: &SignedDirectRequest) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: request.lookup_id.clone(),
        device_id: request.requester.device_id.clone(),
        device_name: request.requester.device_name.clone(),
        public_key: request.requester.public_key.clone(),
        fingerprint: request.requester.fingerprint.clone(),
        node_id: request.requester.node_id.clone(),
        relay_url: "http://127.0.0.1:51821".into(),
        candidates: vec!["127.0.0.1".into()],
        expires_at: 100,
        nonce: "legacy-nonce".into(),
        proof: "legacy-proof".into(),
    }
}

fn connection_pair() -> (SignalConnection, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (peer, _) = listener.accept().unwrap();
    (SignalConnection::from_test_tcp(client).unwrap(), peer)
}

fn read_json(reader: &mut BufReader<TcpStream>) -> serde_json::Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
