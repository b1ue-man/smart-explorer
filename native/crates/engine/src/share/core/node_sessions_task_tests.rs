use std::sync::{Arc, Mutex};

use super::{
    authorize_outgoing_repair_generation, DirectReciprocalTransportResult,
};
use crate::share::core::public_fingerprint;
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::types::ShareAuthState;

#[test]
fn share_remote_task_reciprocal_stale_generation_sends_no_repair_hello() {
    // `repair_direct_reciprocal` runs this gate before resolving a session,
    // opening a stream, or constructing the DirectReciprocal control/Hello.
    let auth = Arc::new(Mutex::new(auth_state(23, true)));
    assert_eq!(
        authorize_outgoing_repair_generation(&auth, 22),
        Err(DirectReciprocalTransportResult::PolicyDenied)
    );
    assert_eq!(authorize_outgoing_repair_generation(&auth, 23), Ok(()));

    auth.lock().unwrap().direct_online = false;
    assert_eq!(
        authorize_outgoing_repair_generation(&auth, 23),
        Err(DirectReciprocalTransportResult::PolicyDenied)
    );
}

fn auth_state(authorization_epoch: u64, direct_online: bool) -> ShareAuthState {
    let identity = share_identity();
    ShareAuthState {
        identity: identity.clone(),
        direct_secret: identity.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online,
        authorization_epoch,
    }
}

fn share_identity() -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[81; 32]);
    let public_key = key.public().to_string();
    ShareIdentity {
        device_id: "local".into(),
        device_name: "Local".into(),
        direct_lookup_id: "local-lookup".into(),
        public_key: public_key.clone(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key,
        iroh_secret: key,
        direct_secret: [82; 32],
    }
}
