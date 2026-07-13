use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tungstenite::Message;

use super::limits::SourceKey;
use super::line::MAX_JSON_LINE;
use super::state::{lock_state, Client, State};
use super::transport::handle;
use super::{tracked_direct, Out, PeerPresence, Writer};

#[test]
fn direct_request_routes_to_lookup_owner() {
    let mut state = State::default();
    let (owner_writer, owner_receiver) = Writer::test_channel(8);
    let (requester_writer, requester_receiver) = Writer::test_channel(8);
    state.clients.insert(
        1,
        client(owner_writer, "owner", HashSet::from(["lookup".into()])),
    );
    state.clients.insert(
        2,
        client(requester_writer.clone(), "requester", HashSet::new()),
    );
    state
        .direct
        .insert("lookup".into(), (1, presence("direct", "lookup", "owner")));
    let state = Arc::new(Mutex::new(state));

    tracked_direct::request_legacy(
        &requester_writer,
        "lookup",
        presence("direct", "lookup", "requester"),
        &state,
    );

    match owner_receiver.recv().unwrap() {
        Out::DirectAccessRequest {
            lookup_id,
            presence,
        } => {
            assert_eq!(lookup_id, "lookup");
            assert_eq!(presence.device_id, "requester");
        }
        _ => panic!("wrong message"),
    }
    assert!(requester_receiver.try_recv().is_err());
}

#[test]
fn direct_accept_routes_to_requester_device() {
    let mut state = State::default();
    let (requester_writer, requester_receiver) = Writer::test_channel(8);
    state.clients.insert(
        1,
        client(requester_writer.clone(), "requester", HashSet::new()),
    );
    let state = Arc::new(Mutex::new(state));

    tracked_direct::decision_legacy(
        &requester_writer,
        "lookup",
        "requester",
        true,
        Some(presence("direct", "lookup", "owner")),
        None,
        &state,
    );

    match requester_receiver.recv().unwrap() {
        Out::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            ..
        } => {
            assert_eq!(lookup_id, "lookup");
            assert_eq!(requester_device_id, "requester");
            assert!(accepted);
            assert_eq!(presence.unwrap().device_id, "owner");
        }
        _ => panic!("wrong message"),
    }
}

#[test]
fn websocket_client_can_hello_and_heartbeat() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle(stream, server_state).unwrap();
    });

    let (mut websocket, _) = tungstenite::connect(format!("ws://{address}/se-share")).unwrap();
    websocket.send(Message::Text(hello_json())).unwrap();
    assert_message_tag(&mut websocket, "hello_ok");

    std::thread::sleep(Duration::from_millis(750));
    websocket
        .send(Message::Text(r#"{"t":"heartbeat"}"#.to_string()))
        .unwrap();
    assert_message_tag(&mut websocket, "pong");

    websocket.close(None).unwrap();
    server.join().unwrap();
}

#[test]
fn websocket_oversized_payload_is_rejected_and_client_is_cleaned_up() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle(stream, server_state)
    });

    let (mut websocket, _) = tungstenite::connect(format!("ws://{address}/se-share")).unwrap();
    websocket.send(Message::Text(hello_json())).unwrap();
    assert_message_tag(&mut websocket, "hello_ok");
    websocket
        .send(Message::Text("x".repeat(MAX_JSON_LINE + 1)))
        .unwrap();
    drop(websocket);

    let error = server.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(lock_state(&state).clients.is_empty());
}

fn client(writer: Writer, device_id: &str, direct_lookup_ids: HashSet<String>) -> Client {
    Client {
        writer,
        source: SourceKey::Ipv4([127, 0, 0, 1]),
        device_id: device_id.into(),
        capabilities: HashSet::new(),
        direct_lookup_ids,
        watched_lookup_ids: HashSet::new(),
        rooms: HashSet::new(),
    }
}

fn presence(kind: &str, relation_id: &str, device_id: &str) -> PeerPresence {
    PeerPresence {
        kind: kind.into(),
        relation_id: relation_id.into(),
        device_id: device_id.into(),
        device_name: device_id.into(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        relay_url: "http://127.0.0.1:51821".into(),
        candidates: vec!["127.0.0.1:1".into()],
        expires_at: 99,
        nonce: "n".into(),
        proof: "proof".into(),
    }
}

fn hello_json() -> String {
    r#"{"t":"hello","protocol_version":3,"device_id":"a","device_name":"Laptop","listen_port":0,"lan":["127.0.0.1"],"public_key":"pk","fingerprint":"fp"}"#.to_string()
}

fn assert_message_tag(
    websocket: &mut tungstenite::WebSocket<
        tungstenite::stream::MaybeTlsStream<std::net::TcpStream>,
    >,
    expected: &str,
) {
    let text = websocket.read().unwrap().into_text().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["t"],
        expected
    );
}
