use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_registry::{ExecAdmission, ExecCancelReason, ExecRegistryLimits};
use crate::share::exec_types::{ExecAuthorization, ExecCommand, ExecId, ExecStart};
use crate::share::identity::ShareIdentity;
use crate::share::types::{
    DirectGrant, DirectGrantState, ExecGrantTarget, RoomMember, RoomProfile, ShareAuthState,
    ShareStatus,
};

#[test]
fn exact_direct_target_enables_then_disable_cancels_and_denies() {
    let (auth, target) = direct_state();
    let registry = ExecRegistry::new(ExecRegistryLimits::default());

    let enabled = mutate(&auth, &registry, target.clone(), true, 10).unwrap();
    assert!(enabled.policy.enabled);
    assert_eq!(enabled.policy.policy_revision, 1);
    assert_eq!(enabled.authorization_epoch, 1);
    let admission = registry
        .prepare(
            enabled.principal.clone(),
            authorization(&enabled, "session-a"),
            &start(),
            11,
        )
        .unwrap();
    let ExecAdmission::Prepared(reservation) = admission else {
        panic!("new execution was not prepared");
    };

    let disabled = mutate(&auth, &registry, target, false, 12).unwrap();
    assert!(!disabled.policy.enabled);
    assert_eq!(disabled.policy.policy_revision, 2);
    assert_eq!(disabled.authorization_epoch, 2);
    assert_eq!(
        reservation.cancellation.reason(),
        Some(ExecCancelReason::Revoked)
    );
    assert!(registry
        .prepare(
            disabled.principal.clone(),
            authorization(&disabled, "session-b"),
            &start(),
            13,
        )
        .is_err());
}

#[test]
fn exact_room_member_policy_is_independent() {
    let (auth, peer) = base_state();
    let room_id = "room-profile".to_string();
    auth.lock().unwrap().rooms.push(RoomProfile {
        id: room_id.clone(),
        name: "Room".into(),
        room_id: "room-relation".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![RoomMember {
            device_id: peer.device_id.clone(),
            device_name: peer.device_name.clone(),
            fingerprint: peer.fingerprint.clone(),
            public_key: peer.public_key.clone(),
            node_id: peer.node_id.clone(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: ShareStatus::Waiting,
            blocked: false,
            exec: ExecGrant::default(),
            presence: None,
        }],
        exports: Default::default(),
    });
    let target = ExecGrantTarget::RoomMember {
        room_id: "room-relation".into(),
        device_id: peer.device_id,
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
    };
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    let mutation = mutate(&auth, &registry, target, true, 20).unwrap();
    assert!(mutation.policy.enabled);
    assert_eq!(mutation.principal.relation_kind, "room");
    assert_eq!(mutation.principal.relation_id, "room-relation");
}

#[test]
fn mismatched_exact_pin_changes_nothing() {
    let (auth, mut target) = direct_state();
    let ExecGrantTarget::Direct { public_key, .. } = &mut target else {
        panic!("wrong target")
    };
    *public_key = "wrong-key".into();
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    assert!(mutate(&auth, &registry, target, true, 10).is_err());
    let state = auth.lock().unwrap();
    assert_eq!(state.authorization_epoch, 0);
    assert!(!state.direct_grants[0].exec.enabled);
    assert_eq!(state.direct_grants[0].exec.policy_revision, 0);
}

#[test]
fn runtime_revision_exhaustion_is_fail_closed() {
    let (auth, target) = direct_state();
    auth.lock().unwrap().direct_grants[0].exec.policy_revision = u64::MAX;
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    assert!(mutate(&auth, &registry, target, true, 10).is_err());
    let state = auth.lock().unwrap();
    assert_eq!(state.authorization_epoch, 0);
    assert!(!state.direct_grants[0].exec.enabled);
}

fn direct_state() -> (Arc<Mutex<ShareAuthState>>, ExecGrantTarget) {
    let (auth, peer) = base_state();
    let target = ExecGrantTarget::Direct {
        device_id: peer.device_id.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
    };
    let mut state = auth.lock().unwrap();
    state.direct_grants.push(DirectGrant {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant::default(),
    });
    drop(state);
    (auth, target)
}

fn base_state() -> (Arc<Mutex<ShareAuthState>>, ShareIdentity) {
    let local = identity("local", "Local", "lookup-local", 11);
    let peer = identity("peer", "Peer", "lookup-peer", 29);
    let state = ShareAuthState {
        identity: local,
        direct_secret: vec![7; 32],
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    };
    (Arc::new(Mutex::new(state)), peer)
}

fn identity(device_id: &str, device_name: &str, lookup_id: &str, key_byte: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[key_byte; 32]);
    let public_key = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_name.into(),
        direct_lookup_id: lookup_id.into(),
        fingerprint: crate::share::core::public_fingerprint(public_key.as_bytes()),
        node_id: public_key.clone(),
        public_key,
        iroh_secret: secret,
        direct_secret: [7; 32],
    }
}

fn authorization(mutation: &ExecGrantMutation, session_id: &str) -> ExecAuthorization {
    ExecAuthorization {
        policy_revision: mutation.policy.policy_revision,
        authorization_epoch: mutation.authorization_epoch,
        session_id: session_id.into(),
    }
}

fn start() -> ExecStart {
    ExecStart {
        exec_id: ExecId::generate().unwrap(),
        command: ExecCommand::Argv {
            program: "echo".into(),
            args: vec!["ok".into()],
        },
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
    }
}
