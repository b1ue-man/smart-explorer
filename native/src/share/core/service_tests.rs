use super::backend::ShareIrohNode;
use super::core::public_fingerprint;
use super::fs::ShareExportConfig;
use super::identity::ShareIdentity;
use super::service::ShareService;
use super::signal_auth::{
    remember_nonce, verify_direct_access_accepted_using, verify_local_direct_request,
};
use super::signal_connection::{normalize_signal_endpoint, normalize_tcp_addr, signal_endpoints};
use super::signal_worker::build_presence;
use super::types::ShareAuthState;
use super::types::{
    DirectAccessState, DirectContact, DirectGrant, RoomProfile, ShareCmd, ShareStatus,
};
use crossbeam_channel::unbounded;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn nonce_cache_detects_replay() {
    let mut seen = HashSet::new();
    let key = "direct:lookup:nonce".to_string();
    assert!(!seen.contains(&key));
    remember_nonce(&mut seen, key.clone());
    assert!(seen.contains(&key));
}

#[test]
fn dropping_probe_clone_does_not_stop_owner_service() {
    let svc = test_service();
    let stopped = svc.stopped.clone();
    let probe_clone = svc.clone();
    drop(probe_clone);
    assert!(!stopped.load(Ordering::Relaxed));
    drop(svc);
    assert!(stopped.load(Ordering::Relaxed));
}

#[test]
fn configure_requires_worker_ack_before_reporting_success() {
    let svc = test_service();
    let contact = DirectContact {
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
    };
    let room = RoomProfile {
        id: "room-profile-a".into(),
        name: "Room A".into(),
        room_id: "room-a".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: Vec::new(),
        exports: ShareExportConfig::default(),
    };
    assert!(svc
        .cmd(ShareCmd::Configure {
            direct: vec![contact],
            direct_grants: Vec::new(),
            rooms: vec![room],
            default_direct_exports: ShareExportConfig::default(),
        })
        .is_err());
    assert!(svc.auth.lock().unwrap().direct_contacts.is_empty());
    assert!(svc.auth.lock().unwrap().rooms.is_empty());
}

#[test]
fn signal_endpoint_config_supports_https_and_fallbacks() {
    assert_eq!(
        signal_endpoints(" wss://share.example/ws ; 10.0.0.5:51820 "),
        vec![
            "wss://share.example/ws".to_string(),
            "10.0.0.5:51820".to_string()
        ]
    );
    assert_eq!(
        normalize_signal_endpoint("https://share.example/ws"),
        "wss://share.example/ws"
    );
    assert_eq!(
        normalize_signal_endpoint("http://share.example/ws"),
        "ws://share.example/ws"
    );
}

#[test]
fn tcp_endpoint_defaults_to_share_port() {
    assert_eq!(normalize_tcp_addr("share.example"), "share.example:51820");
    assert_eq!(normalize_tcp_addr("share.example:443"), "share.example:443");
    assert_eq!(normalize_tcp_addr("[::1]:51820"), "[::1]:51820");
}

#[test]
fn local_direct_request_requires_own_direct_secret() {
    let svc = test_service();
    let identity = svc.identity.clone();
    let secret = svc.auth.lock().unwrap().direct_secret.clone();
    let presence = build_presence(
        "direct",
        &identity.direct_lookup_id,
        &identity,
        &secret,
        &svc.iroh,
    )
    .unwrap();
    assert!(verify_local_direct_request(
        &identity.direct_lookup_id,
        &presence,
        &svc.auth
    ));

    let wrong = build_presence(
        "direct",
        &identity.direct_lookup_id,
        &identity,
        &[9u8; 32],
        &svc.iroh,
    )
    .unwrap();
    assert!(!verify_local_direct_request(
        &identity.direct_lookup_id,
        &wrong,
        &svc.auth
    ));
}

#[test]
fn direct_accept_or_reject_requires_signed_owner_presence() {
    let svc = test_service();
    let secret = vec![7u8; 32];
    let owner = test_identity("owner", "Owner", "lookup-owner");
    let contact = DirectContact {
        id: "contact-owner".into(),
        display_name: "Owner".into(),
        lookup_id: "lookup-owner".into(),
        expected_fingerprint: owner.fingerprint.clone(),
        expected_node_id: owner.node_id.clone(),
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::WaitingForAccess,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
    };
    svc.auth.lock().unwrap().direct_contacts = vec![contact];
    let signed = build_presence("direct", "lookup-owner", &owner, &secret, &svc.iroh).unwrap();
    assert!(verify_direct_access_accepted_using(
        "lookup-owner",
        &svc.identity.device_id,
        Some(&signed),
        &svc.auth,
        |_| Some(secret.clone())
    ));
    assert!(!verify_direct_access_accepted_using(
        "lookup-owner",
        &svc.identity.device_id,
        None,
        &svc.auth,
        |_| Some(secret.clone())
    ));
    let wrong = build_presence("direct", "lookup-owner", &owner, &[9u8; 32], &svc.iroh).unwrap();
    assert!(!verify_direct_access_accepted_using(
        "lookup-owner",
        &svc.identity.device_id,
        Some(&wrong),
        &svc.auth,
        |_| Some(secret.clone())
    ));
}

#[test]
fn presence_binds_node_id_and_relay_url() {
    let svc = test_service();
    let relation_id = "lookup-owner";
    let secret = svc.auth.lock().unwrap().direct_secret.clone();
    let owner = test_identity("owner", "Owner", relation_id);
    let contact = DirectContact {
        id: "contact-owner".into(),
        display_name: "Owner".into(),
        lookup_id: relation_id.into(),
        expected_fingerprint: owner.fingerprint.clone(),
        expected_node_id: owner.node_id.clone(),
        remote_device_id: None,
        remote_public_key: None,
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::WaitingForAccess,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Pending,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
    };
    svc.auth.lock().unwrap().direct_contacts = vec![contact];
    let presence = build_presence("direct", relation_id, &owner, &secret, &svc.iroh).unwrap();
    let mut tampered = presence.clone();
    tampered.node_id.push('x');
    assert!(!verify_direct_access_accepted_using(
        relation_id,
        &svc.identity.device_id,
        Some(&tampered),
        &svc.auth,
        |_| Some(secret.clone())
    ));
    assert!(verify_direct_access_accepted_using(
        relation_id,
        &svc.identity.device_id,
        Some(&presence),
        &svc.auth,
        |_| Some(secret.clone())
    ));
}

fn test_service() -> ShareService {
    let (cmd_tx, _cmd_rx) = unbounded();
    let (ev_tx, ev_rx) = unbounded();
    let identity = test_identity("device-a", "Device A", "lookup-local");
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: identity.clone(),
        direct_secret: vec![0u8; 32],
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::<DirectGrant>::new(),
        rooms: Vec::new(),
        seen_nonces: HashSet::new(),
        direct_online: true,
    }));
    let iroh = ShareIrohNode::start("127.0.0.1:0", &identity, auth.clone(), ev_tx).unwrap();
    ShareService {
        events: ev_rx,
        cmds: cmd_tx,
        identity,
        listen_port: 0,
        auth,
        iroh,
        stopped: Arc::new(AtomicBool::new(false)),
        server: "127.0.0.1:0".into(),
        owner: true,
    }
}

fn test_identity(device_id: &str, device_name: &str, lookup: &str) -> ShareIdentity {
    let mut secret_bytes = [7u8; 32];
    for (index, byte) in device_id.bytes().enumerate() {
        let slot = index % secret_bytes.len();
        secret_bytes[slot] = secret_bytes[slot].wrapping_mul(31).wrapping_add(byte);
    }
    let secret = iroh::SecretKey::from_bytes(&secret_bytes);
    let node_id = secret.public().to_string();
    let fingerprint = public_fingerprint(node_id.as_bytes());
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_name.into(),
        direct_lookup_id: lookup.into(),
        public_key: node_id.clone(),
        fingerprint,
        node_id,
        iroh_secret: secret,
        direct_secret: [0; 32],
    }
}
