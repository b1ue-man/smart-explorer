//! Z-H1: the per-session proof memo never outlives a change of the state it
//! was verified against. Every mutation that matters for authorization fails
//! the next check, without any revision counter to forget.
use std::sync::{Arc, Mutex};

use crate::share::core::{hmac_proof, public_fingerprint};
use crate::share::exec_policy::ExecGrant;
use crate::share::identity::ShareIdentity;
use crate::share::session::{authenticate_incoming_session, session_payload, IncomingSession};
use crate::share::types::{DirectGrant, DirectGrantState, ShareAuthState};
use crate::share::wire::PeerHello;

fn identity(device_id: &str, seed: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_id.into(),
        direct_lookup_id: format!("lookup-{device_id}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: [seed; 32],
    }
}

fn grant(client: &ShareIdentity, write: bool) -> DirectGrant {
    DirectGrant {
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant::default(),
        write,
    }
}

fn state(host: &ShareIdentity, client: &ShareIdentity) -> ShareAuthState {
    ShareAuthState {
        identity: host.clone(),
        direct_secret: host.direct_secret(),
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![grant(client, true)],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    }
}

fn session(
    host: &ShareIdentity,
    client: &ShareIdentity,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> IncomingSession {
    let nonce = "memo-nonce";
    let payload = session_payload(
        "direct",
        &host.direct_lookup_id,
        &client.device_id,
        &host.device_id,
        &client.node_id,
        &host.node_id,
        nonce,
    );
    let hello = PeerHello {
        protocol_version: 3,
        relation_kind: "direct".into(),
        relation_id: host.direct_lookup_id.clone(),
        device_id: client.device_id.clone(),
        public_key: client.public_key.clone(),
        node_id: client.node_id.clone(),
        session_nonce: nonce.into(),
        session_proof: hmac_proof(&host.direct_secret(), &payload),
        requested_capabilities: Vec::new(),
    };
    authenticate_incoming_session(&hello, &client.node_id, auth).unwrap()
}

fn fixture() -> (Arc<Mutex<ShareAuthState>>, IncomingSession) {
    let host = identity("host", 11);
    let client = identity("client", 12);
    let auth = Arc::new(Mutex::new(state(&host, &client)));
    let session = session(&host, &client, &auth);
    (auth, session)
}

#[test]
fn speed_task_share_auth_memo_is_filled_by_the_first_check() {
    let (auth, session) = fixture();
    // `authenticate_incoming_session` verified the proof once.
    assert!(!session.memo_is_empty());
    assert!(session.may_write(&auth).unwrap());
    assert!(session.authorize(&auth).is_ok());
}

#[test]
fn speed_task_share_auth_revoked_grant_fails_the_next_check() {
    let (auth, session) = fixture();
    assert!(session.may_write(&auth).unwrap());
    auth.lock().unwrap().direct_grants[0].state = DirectGrantState::Ignored;
    assert!(session.may_write(&auth).is_err());
    assert!(session.authorize(&auth).is_err());
    auth.lock().unwrap().direct_grants.clear();
    assert!(session.may_write(&auth).is_err());
}

#[test]
fn speed_task_share_auth_write_right_follows_the_current_grant() {
    let (auth, session) = fixture();
    assert!(session.may_write(&auth).unwrap());
    auth.lock().unwrap().direct_grants[0].write = false;
    assert!(!session.may_write(&auth).unwrap());
    auth.lock().unwrap().direct_grants[0].write = true;
    assert!(session.may_write(&auth).unwrap());
}

#[test]
fn speed_task_share_auth_offline_and_secret_change_fail_the_next_check() {
    let (auth, session) = fixture();
    auth.lock().unwrap().direct_online = false;
    assert!(session.may_write(&auth).is_err());
    auth.lock().unwrap().direct_online = true;
    assert!(session.may_write(&auth).is_ok());
    // A rotated relation secret no longer verifies the session's proof: the
    // memo is bound to the exact secret bytes.
    auth.lock().unwrap().direct_secret = vec![99; 32];
    assert!(session.may_write(&auth).is_err());
    assert!(session.authorize(&auth).is_err());
}

#[test]
fn speed_task_share_auth_changed_peer_key_material_recomputes() {
    let (auth, session) = fixture();
    // A grant whose fingerprint no longer matches its key fails the full
    // check the changed memo input forces.
    auth.lock().unwrap().direct_grants[0].fingerprint = "00".repeat(32);
    assert!(session.may_write(&auth).is_err());
}

#[test]
fn speed_task_share_auth_changed_local_identity_recomputes() {
    let (auth, session) = fixture();
    // The proof is bound to the host's device id: another one fails.
    auth.lock().unwrap().identity.device_id = "replaced-host".into();
    assert!(session.may_write(&auth).is_err());
}
