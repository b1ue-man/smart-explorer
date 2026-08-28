use std::sync::{Arc, Mutex};

use super::mutate_exec_grant;
use crate::share::core::public_fingerprint;
use crate::share::direct_reciprocal_coordinator::DirectReciprocalCoordinator;
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::node::ShareIrohNode;
use crate::share::types::{
    DirectGrant, DirectGrantState, ExecGrantTarget, ShareAuthState,
};

#[test]
fn share_remote_task_reciprocal_exec_grant_epoch_resynchronizes_coordinator() {
    let local = share_identity(91, "local", "local-lookup", 92);
    let remote_key = iroh::SecretKey::from_bytes(&[93; 32]);
    let remote = DirectPeerIdentity::from_secret("remote", "Remote", &remote_key);
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: local.clone(),
        direct_secret: local.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![direct_grant(&remote)],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: false,
        authorization_epoch: 31,
    }));
    let (events, _receiver) = crossbeam_channel::unbounded();
    let node = ShareIrohNode::start("", &local, auth.clone(), events).unwrap();
    let coordinator = Arc::new(DirectReciprocalCoordinator::detached_for_task_test(31));
    node.install_direct_repair_coordinator(&coordinator).unwrap();

    let mutation = mutate_exec_grant(
        &auth,
        &node,
        ExecGrantTarget::Direct {
            device_id: remote.device_id.clone(),
            public_key: remote.public_key.clone(),
            fingerprint: remote.fingerprint.clone(),
            node_id: remote.node_id.clone(),
        },
        true,
    )
    .unwrap();
    assert_eq!(mutation.authorization_epoch, 32);
    assert_eq!(auth.lock().unwrap().authorization_epoch, 32);
    assert_eq!(coordinator.generation_for_task_test(), 32);
    assert_eq!(coordinator.task_count_for_task_test(), 0);

    node.stop_sharing().unwrap();
}

fn share_identity(seed: u8, device_id: &str, lookup_id: &str, secret: u8) -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let public_key = key.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_id.into(),
        direct_lookup_id: lookup_id.into(),
        public_key: public_key.clone(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key,
        iroh_secret: key,
        direct_secret: [secret; 32],
    }
}

fn direct_grant(identity: &DirectPeerIdentity) -> DirectGrant {
    DirectGrant {
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
    }
}
