use std::fs;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use crossbeam_channel::unbounded;
use iroh::endpoint::ReadExactError;

use super::backend::{PeerBackend, ShareIrohNode};
use super::core::public_fingerprint;
use super::framing::read_exact_error;
use super::fs::{ShareExportConfig, SharedRoot};
use super::identity::ShareIdentity;
use super::session::relay_url_from_signal;
use super::types::{
    DirectAccessState, DirectContact, DirectGrantState, PeerEndpoint, PeerOpenTarget, PeerPresence,
    ShareAuthState, ShareScope, ShareStatus,
};
use crate::vfs::Backend;

#[test]
fn remote_drive_task_iroh_mount_reconnects_without_losing_lease() {
    let secret = vec![7u8; 32];
    let root = std::env::temp_dir().join(format!(
        "se-iroh-direct-{}-{}",
        std::process::id(),
        crate::share::core_now_secs()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hello.txt"), b"hello from gate").unwrap();

    let a = test_identity("device-a", "Device A", "lookup-a");
    let b = test_identity("device-b", "Device B", "lookup-b");
    assert_ne!(a.node_id, b.node_id, "test peers need distinct Iroh keys");
    let (tx_a, _events_a) = unbounded();
    let (tx_b, _events_b) = unbounded();

    let auth_b = Arc::new(Mutex::new(ShareAuthState {
        identity: b.clone(),
        direct_secret: secret.clone(),
        default_direct_exports: ShareExportConfig {
            roots: vec![SharedRoot {
                label: "Gate".into(),
                path: root.to_string_lossy().replace('\\', "/"),
            }],
            include_connections: false,
        },
        direct_contacts: Vec::new(),
        direct_grants: vec![crate::share::types::DirectGrant {
            device_id: a.device_id.clone(),
            device_name: a.device_name.clone(),
            public_key: a.public_key.clone(),
            fingerprint: a.fingerprint.clone(),
            node_id: a.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: 1,
            exec: crate::share::ExecGrant::default(),
        }],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }));
    let auth_a = Arc::new(Mutex::new(ShareAuthState {
        identity: a.clone(),
        direct_secret: vec![0u8; 32],
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }));

    let node_b = ShareIrohNode::start("relay-disabled://test", &b, auth_b, tx_b).unwrap();
    let node_a = ShareIrohNode::start("relay-disabled://test", &a, auth_a.clone(), tx_a).unwrap();
    let mut candidates = node_b.candidates();
    if let Some(port) = candidates
        .iter()
        .filter_map(|candidate| candidate.parse::<std::net::SocketAddr>().ok())
        .next()
        .map(|addr| addr.port())
    {
        candidates.insert(0, format!("127.0.0.1:{port}"));
    }
    let presence = PeerPresence {
        kind: "direct".into(),
        relation_id: b.direct_lookup_id.clone(),
        device_id: b.device_id.clone(),
        device_name: b.device_name.clone(),
        public_key: b.public_key.clone(),
        fingerprint: b.fingerprint.clone(),
        node_id: b.node_id.clone(),
        relay_url: String::new(),
        candidates,
        expires_at: crate::share::core_now_secs() + 300,
        nonce: "test".into(),
        proof: String::new(),
    };
    let endpoint = PeerEndpoint {
        label: "test".into(),
        scope: ShareScope::Direct {
            contact_id: "contact-b".into(),
        },
        presence,
        relation_secret: secret,
        expected_node_id: Some(b.node_id.clone()),
    };
    auth_a.lock().unwrap().direct_contacts = vec![DirectContact {
        id: "contact-b".into(),
        display_name: b.device_name.clone(),
        lookup_id: b.direct_lookup_id.clone(),
        expected_fingerprint: b.fingerprint.clone(),
        expected_node_id: b.node_id.clone(),
        remote_device_id: Some(b.device_id.clone()),
        remote_public_key: Some(b.public_key.clone()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Available,
        last_error: None,
        presence: Some(endpoint.presence.clone()),
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(b.public_key.clone()),
    }];
    let mut stale_initial = endpoint.clone();
    stale_initial.presence.expires_at = crate::share::core_now_secs() - 1;
    let backend = PeerBackend::new_live(
        stale_initial,
        PeerOpenTarget::Direct {
            contact_id: "contact-b".into(),
        },
        auth_a.clone(),
        a.clone(),
        node_a.clone(),
    );

    backend.mount_path_capabilities("/Gate").unwrap();
    let original_generation = node_a
        .outgoing_generation_for_test(backend.initial_endpoint())
        .unwrap()
        .expect("capability probe must establish one reusable session");

    let root_entries = backend.list_dir("/Gate").unwrap();
    assert!(root_entries
        .iter()
        .any(|entry| entry.name == "hello.txt" && !entry.is_dir));
    let mut text = String::new();
    backend
        .open_read("/Gate/hello.txt")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "hello from gate");

    let files_seen = std::sync::atomic::AtomicU64::new(0);
    let bytes_seen = std::sync::atomic::AtomicU64::new(0);
    let tree = backend
        .walk_tree("/Gate", &|files, bytes| {
            files_seen.store(files, std::sync::atomic::Ordering::Relaxed);
            bytes_seen.store(bytes, std::sync::atomic::Ordering::Relaxed);
            true
        })
        .unwrap()
        .unwrap();
    assert_eq!(tree.name, "Gate");
    assert_eq!(tree.size, 15);
    assert_eq!(files_seen.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(bytes_seen.load(std::sync::atomic::Ordering::Relaxed), 15);
    let canceled = backend
        .walk_tree("/Gate", &|files, _| files == 0)
        .unwrap_err();
    assert_eq!(canceled.kind(), io::ErrorKind::Interrupted);

    {
        let mut writer = backend.open_write("/Gate/new.txt").unwrap();
        writer.write_all(b"written over iroh").unwrap();
        writer.flush().unwrap();
    }
    assert_eq!(
        fs::read(root.join("new.txt")).unwrap(),
        b"written over iroh"
    );
    {
        let mut writer = backend.open_write("/Gate/new.txt").unwrap();
        writer.write_all(b"partial replacement").unwrap();
    }
    for _ in 0..50 {
        let staged = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains(".se-part-"));
        if !staged {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        fs::read(root.join("new.txt")).unwrap(),
        b"written over iroh"
    );
    assert_eq!(
        backend
            .copy_file("/Gate/new.txt", "/Gate/copy.txt")
            .unwrap(),
        17
    );
    backend
        .rename("/Gate/copy.txt", "/Gate/renamed.txt")
        .unwrap();
    assert!(root.join("renamed.txt").exists());
    backend.remove_file("/Gate/renamed.txt").unwrap();
    assert!(!root.join("renamed.txt").exists());
    assert_eq!(
        node_a
            .outgoing_generation_for_test(backend.initial_endpoint())
            .unwrap(),
        Some(original_generation),
        "filesystem calls must multiplex over the mounted session",
    );

    {
        let mut state = auth_a.lock().unwrap();
        let refreshed = state.direct_contacts[0].presence.as_mut().unwrap();
        refreshed.expires_at = crate::share::core_now_secs() + 300;
        refreshed.nonce = "refreshed-route".into();
    }
    assert!(node_a
        .disconnect_outgoing_for_test(backend.initial_endpoint())
        .unwrap());
    assert_eq!(backend.stat("/Gate/new.txt").unwrap().size, 17);
    assert_ne!(
        node_a
            .outgoing_generation_for_test(backend.initial_endpoint())
            .unwrap(),
        Some(original_generation),
        "a physical reconnect must replace the transport without remounting",
    );

    node_b.invalidate_sessions().unwrap();
    let revoked = backend.stat("/Gate/new.txt").unwrap_err();
    assert_eq!(revoked.kind(), io::ErrorKind::PermissionDenied);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn relay_url_tracks_signal_transport() {
    assert_eq!(
        relay_url_from_signal("tcp://127.0.0.1:51820"),
        "http://127.0.0.1:51821"
    );
    assert_eq!(
        relay_url_from_signal("127.0.0.1:51820"),
        "http://127.0.0.1:51821"
    );
    assert_eq!(
        relay_url_from_signal("wss://share.example/se-share"),
        "https://share.example"
    );
    assert_eq!(
        relay_url_from_signal("https://share.example/se-share"),
        "https://share.example"
    );
}

#[test]
fn early_peer_close_is_reported_as_unexpected_eof() {
    let error = read_exact_error(ReadExactError::FinishedEarly(3));
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert!(error.to_string().contains("3 bytes"));
}

fn test_identity(device_id: &str, device_name: &str, lookup: &str) -> ShareIdentity {
    let mut secret_bytes = [11u8; 32];
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
