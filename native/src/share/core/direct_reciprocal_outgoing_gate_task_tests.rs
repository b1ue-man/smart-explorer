use super::*;
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::types::{PeerPresence, ShareStatus};

fn gate() -> OutgoingRepairPersistGate {
    let local_key = iroh::SecretKey::from_bytes(&[3; 32]);
    let local = DirectPeerIdentity::from_secret("local", "Local", &local_key);
    let remote = DirectPeerIdentity::from_secret("remote", "Remote", &iroh::SecretKey::from_bytes(&[4; 32]));
    let identity = ShareIdentity {
        device_id: local.device_id,
        device_name: local.device_name,
        public_key: local.public_key,
        fingerprint: local.fingerprint,
        node_id: local.node_id,
        direct_lookup_id: "local-lookup".into(),
        iroh_secret: local_key,
        direct_secret: [7; 32],
    };
    let contact = DirectContact {
        id: "c".into(),
        display_name: "Remote".into(),
        lookup_id: "remote-lookup".into(),
        expected_fingerprint: remote.fingerprint.clone(),
        expected_node_id: remote.node_id.clone(),
        remote_device_id: Some(remote.device_id.clone()),
        remote_public_key: Some(remote.public_key.clone()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(remote.public_key.clone()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    };
    let endpoint = PeerEndpoint {
        label: "Remote".into(),
        scope: ShareScope::Direct { contact_id: "c".into() },
        presence: PeerPresence {
            kind: "direct".into(),
            relation_id: "remote-lookup".into(),
            device_id: remote.device_id,
            device_name: remote.device_name,
            public_key: remote.public_key,
            fingerprint: remote.fingerprint,
            node_id: remote.node_id,
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 100,
            nonce: "nonce".into(),
            proof: String::new(),
        },
        relation_secret: vec![8; 32],
        expected_node_id: Some(contact.expected_node_id.clone()),
    };
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: identity.clone(),
        direct_secret: identity.direct_secret(),
        default_direct_exports: Default::default(),
        direct_contacts: vec![contact],
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    }));
    OutgoingRepairPersistGate { transition_slot: Arc::new(Semaphore::new(1)), auth, identity, endpoint }
}

#[test]
fn review_task_outgoing_store_gate_uses_current_peer_pins_without_global_epoch_equality() {
    let gate = gate();
    assert!(gate.authorize_using(|_| Some(vec![8; 32])).is_ok());
    gate.auth.lock().unwrap().authorization_epoch += 1;
    assert!(gate.authorize_using(|_| Some(vec![8; 32])).is_ok());
    gate.auth.lock().unwrap().direct_contacts[0].access_state = DirectAccessState::Ignored;
    assert!(matches!(gate.authorize_using(|_| Some(vec![8; 32])), Err(DirectRepairSessionError::PolicyDenied)));
    gate.auth.lock().unwrap().direct_contacts[0].access_state = DirectAccessState::Accepted;
    gate.auth.lock().unwrap().direct_contacts[0].remote_public_key = Some("replaced-key".into());
    assert!(gate.authorize_using(|_| Some(vec![8; 32])).is_err());
}

#[test]
fn review_task_outgoing_repair_yields_immediately_to_configuration() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let gate = gate();
        let slots = gate.transition_slot.clone();
        let held = slots.clone().try_acquire_owned().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), gate.acquire()).await;
        assert!(matches!(result.unwrap(), Err(DirectRepairSessionError::Store(DirectRepairStoreError::Retryable))));
        drop(held);
        assert_eq!(slots.available_permits(), 1);
    });
}
