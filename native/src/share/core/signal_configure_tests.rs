use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::unbounded;

use super::backend::ShareIrohNode;
use super::core::public_fingerprint;
use super::fs::ShareExportConfig;
use super::identity::ShareIdentity;
use super::signal_commands::{run_connected_command, ConnectedCommandRuntime};
use super::signal_connection::SignalConnection;
use super::signal_worker::publish_all;
use super::types::{
    DirectAccessState, DirectContact, DirectGrant, RoomProfile, ShareAuthState, ShareCmd,
    ShareStatus,
};

#[test]
fn configure_emits_real_teardown_before_republishing_new_state() {
    let fixture = fixture();
    let (mut signal, peer) = connection_pair();
    let mut sent = HashSet::new();
    let mut attempts = HashMap::new();
    let (events, _) = unbounded();
    let mut runtime = ConnectedCommandRuntime {
        signal: &mut signal,
        auth: &fixture.auth,
        iroh: &fixture.iroh,
        direct_requests_sent: &mut sent,
        tracked_direct: true,
        events: &events,
        tracked_attempts: &mut attempts,
    };

    let outcome = run_connected_command(empty_configuration(), &mut runtime);

    assert!(outcome.result.is_ok());
    assert!(!outcome.should_reconnect);
    let mut reader = BufReader::new(peer);
    assert_eq!(message_type(read_line(&mut reader)), "unwatch_direct");
    assert_eq!(message_type(read_line(&mut reader)), "leave_room");
    assert_eq!(message_type(read_line(&mut reader)), "publish_direct");
}

#[test]
fn configure_write_failure_forces_reconnect_and_new_state_has_no_old_subscriptions() {
    let fixture = fixture();
    let (mut broken, peer) = connection_pair();
    drop(peer);
    broken.shutdown_test_transport().unwrap();
    let mut sent = HashSet::new();
    let mut attempts = HashMap::new();
    let (events, _) = unbounded();
    let mut runtime = ConnectedCommandRuntime {
        signal: &mut broken,
        auth: &fixture.auth,
        iroh: &fixture.iroh,
        direct_requests_sent: &mut sent,
        tracked_direct: true,
        events: &events,
        tracked_attempts: &mut attempts,
    };

    let outcome = run_connected_command(empty_configuration(), &mut runtime);

    assert!(outcome.result.is_err());
    assert!(outcome.should_reconnect);
    assert!(fixture.auth.lock().unwrap().direct_contacts.is_empty());
    assert!(fixture.auth.lock().unwrap().rooms.is_empty());

    let (mut reconnected, peer) = connection_pair();
    publish_all(
        &mut reconnected,
        &fixture.auth,
        &fixture.iroh,
        &mut sent,
        true,
    )
    .unwrap();
    let mut reader = BufReader::new(peer);
    assert_eq!(message_type(read_line(&mut reader)), "publish_direct");
    assert!(read_optional_line(&mut reader).is_none());
}

struct Fixture {
    auth: Arc<Mutex<ShareAuthState>>,
    iroh: Arc<ShareIrohNode>,
}

fn fixture() -> Fixture {
    let secret = iroh::SecretKey::from_bytes(&[7; 32]);
    let public_key = secret.public().to_string();
    let identity = ShareIdentity {
        device_id: "local-device".into(),
        device_name: "Local Device".into(),
        direct_lookup_id: "local-lookup".into(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key.clone(),
        public_key,
        iroh_secret: secret,
        direct_secret: [8; 32],
    };
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: identity.clone(),
        direct_secret: vec![8; 32],
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: vec![contact()],
        direct_grants: Vec::<DirectGrant>::new(),
        rooms: vec![room()],
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: HashSet::new(),
        direct_online: true,
        authorization_epoch: 0,
    }));
    let (events, _) = unbounded();
    let iroh = ShareIrohNode::start("127.0.0.1:0", &identity, auth.clone(), events).unwrap();
    Fixture { auth, iroh }
}

fn empty_configuration() -> ShareCmd {
    ShareCmd::Configure {
        direct: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        default_direct_exports: ShareExportConfig::default(),
    }
}

fn contact() -> DirectContact {
    DirectContact {
        id: "contact-a".into(),
        display_name: "A".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: "00".repeat(16),
        expected_node_id: "node-a".into(),
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
    }
}

fn room() -> RoomProfile {
    RoomProfile {
        id: "room-profile-a".into(),
        name: "Room A".into(),
        room_id: "room-a".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: Vec::new(),
        exports: ShareExportConfig::default(),
    }
}

fn connection_pair() -> (SignalConnection, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    (SignalConnection::from_test_tcp(client).unwrap(), peer)
}

fn read_line(reader: &mut BufReader<TcpStream>) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    line
}

fn read_optional_line(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

fn message_type(line: String) -> String {
    serde_json::from_str::<serde_json::Value>(&line).unwrap()["t"]
        .as_str()
        .unwrap()
        .to_string()
}
