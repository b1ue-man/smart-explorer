//! The Iroh endpoint of a Share node resolves relay host names through the
//! operating system, and a relay that stays unreachable reaches the worker
//! log (Windows devices without a relay, 2026-10-22).
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ShareIrohNode;
use crate::share::core::public_fingerprint;
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::types::{ShareAuthState, ShareEvent};

fn identity() -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[71; 32]);
    let public_key = key.public().to_string();
    ShareIdentity {
        device_id: "relay-test".into(),
        device_name: "relay-test".into(),
        direct_lookup_id: "relay-test-lookup".into(),
        public_key: public_key.clone(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key,
        iroh_secret: key,
        direct_secret: [72; 32],
    }
}

fn auth(identity: &ShareIdentity) -> Arc<Mutex<ShareAuthState>> {
    Arc::new(Mutex::new(ShareAuthState {
        identity: identity.clone(),
        direct_secret: identity.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }))
}

#[test]
fn windows_direct_task_endpoint_resolves_relays_through_the_system() {
    let identity = identity();
    let (events, _receiver) = crossbeam_channel::unbounded();
    let node = ShareIrohNode::start("127.0.0.1:0", &identity, auth(&identity), events).unwrap();
    let described = format!("{:?}", node.endpoint.dns_resolver().unwrap());
    assert!(described.contains("SystemFirstResolver"), "{described}");
}

#[test]
fn windows_direct_task_unreachable_relay_is_named_in_the_log() {
    let identity = identity();
    let (events, receiver) = crossbeam_channel::unbounded();
    // `.invalid` never resolves (RFC 6761): no relay can be reached.
    let node = ShareIrohNode::start(
        "wss://relay.invalid:443",
        &identity,
        auth(&identity),
        events,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let warning = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(ShareEvent::Error(message)) if message.contains("Relay nicht erreichbar") => {
                break message
            }
            Ok(_) => continue,
            Err(error) => panic!("no relay warning: {error}"),
        }
    };
    assert!(warning.contains("relay.invalid"), "{warning}");
    drop(node);
}
