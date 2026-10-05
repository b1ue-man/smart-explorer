//! Normal room configuration transitions retain default deny and revocation.
use super::*;
use crate::share::exec_registry::{ExecAdmission, ExecCancelReason, ExecRegistryLimits};
use crate::share::exec_types::{ExecAuthorization, ExecCommand, ExecId, ExecStart};
use crate::share::identity::ShareIdentity;
use crate::share::types::{RoomMember, RoomProfile, ShareStatus};

fn room_state(revision: u64, enabled: bool) -> ShareAuthState {
    let identity = |byte, id: &str| {
        let secret = iroh::SecretKey::from_bytes(&[byte; 32]);
        let public_key = secret.public().to_string();
        ShareIdentity {
            device_id: id.into(),
            device_name: id.into(),
            direct_lookup_id: id.into(),
            fingerprint: crate::share::core::public_fingerprint(public_key.as_bytes()),
            node_id: public_key.clone(),
            public_key,
            iroh_secret: secret,
            direct_secret: [7; 32],
        }
    };
    let peer = identity(29, "peer");
    ShareAuthState {
        identity: identity(11, "local"),
        direct_secret: vec![7; 32],
        default_direct_exports: Default::default(),
        direct_contacts: vec![],
        direct_grants: vec![],
        direct_requests: vec![],
        direct_request_tombstones: vec![],
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
        rooms: vec![RoomProfile {
            id: "room-profile".into(),
            name: "Room".into(),
            room_id: "room-relation".into(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            exports: Default::default(),
            policy: crate::share::RoomPolicy::new_room(),
            members: vec![RoomMember {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                public_key: peer.public_key.clone(),
                fingerprint: peer.fingerprint.clone(),
                node_id: peer.node_id.clone(),
                relay_url: String::new(),
                candidates: vec![],
                last_seen: None,
                status: ShareStatus::Waiting,
                blocked: false,
                presence: None,
                relation: Default::default(),
                exec: ExecGrant {
                    enabled,
                    policy_revision: revision,
                    ..ExecGrant::default()
                },
            }],
        }],
    }
}

fn request() -> ExecStart {
    ExecStart {
        exec_id: ExecId::generate().unwrap(),
        command: ExecCommand::Argv {
            program: "echo".into(),
            args: vec!["ok".into()],
        },
        cwd: None,
        env: Default::default(),
        timeout_ms: None,
        max_output_bytes: None,
    }
}

fn token(revision: u64, epoch: u64) -> ExecAuthorization {
    ExecAuthorization {
        policy_revision: revision,
        authorization_epoch: epoch,
        session_id: "session".into(),
    }
}

#[test]
fn sync_reliability_task_provider_room_default_deny_reappears() {
    let current = room_state(0, false);
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    apply_configuration_transition(&current, &current, 0, &registry).unwrap();
    let mut missing = current.clone();
    missing.rooms[0].members.clear();
    missing.authorization_epoch = 1;
    apply_configuration_transition(&current, &missing, 1, &registry).unwrap();
    let mut restored = current.clone();
    restored.authorization_epoch = 1;
    apply_configuration_transition(&missing, &restored, 1, &registry).unwrap();
    let principal = effective_policies(&restored).remove(0).principal;
    assert!(registry
        .prepare(principal.clone(), token(0, 1), &request(), 10)
        .is_err());
    // Only a fresh explicit grant may enable Exec after the normal refresh.
    let target = ExecGrantTarget::RoomMember {
        room_id: principal.relation_id,
        device_id: principal.device_id,
        public_key: principal.public_key,
        fingerprint: principal.fingerprint,
        node_id: principal.node_id,
    };
    let mutation = mutate(&Arc::new(Mutex::new(restored)), &registry, target, true, 11).unwrap();
    assert!(registry
        .prepare(mutation.principal, token(1, 2), &request(), 12)
        .is_ok());
}

#[test]
fn sync_reliability_task_provider_room_enabled_removal_revokes() {
    let current = room_state(1, true);
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    apply_configuration_transition(&current, &current, 0, &registry).unwrap();
    let principal = effective_policies(&current).remove(0).principal;
    let ExecAdmission::Prepared(reservation) = registry
        .prepare(principal.clone(), token(1, 0), &request(), 10)
        .unwrap()
    else {
        panic!("expected owned launch reservation")
    };
    let mut removed = current.clone();
    removed.rooms[0].members.clear();
    removed.authorization_epoch = 1;
    apply_configuration_transition(&current, &removed, 1, &registry).unwrap();
    assert_eq!(
        reservation.cancellation.reason(),
        Some(ExecCancelReason::Revoked)
    );
    let mut stale = current.clone();
    stale.authorization_epoch = 1;
    assert!(apply_configuration_transition(&removed, &stale, 1, &registry).is_err());
    // The removal's synthetic revision is 2: enabling that same revision is
    // still forbidden; only a strictly newer explicit decision may enable.
    stale.rooms[0].members[0].exec.policy_revision = 2;
    assert!(apply_configuration_transition(&removed, &stale, 1, &registry).is_err());
    stale.rooms[0].members[0].exec.policy_revision = 3;
    apply_configuration_transition(&removed, &stale, 1, &registry).unwrap();
    assert!(registry
        .commit_start(&reservation.lease, || Ok(()))
        .is_err());
    assert!(registry
        .prepare(principal, token(3, 1), &request(), 12)
        .is_ok());
}

#[test]
fn sync_reliability_task_provider_rejected_refresh_preserves_epoch_and_launch() {
    let current = room_state(2, true);
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    apply_configuration_transition(&current, &current, 0, &registry).unwrap();
    let principal = effective_policies(&current).remove(0).principal;
    let ExecAdmission::Prepared(reservation) = registry
        .prepare(principal, token(2, 0), &request(), 10)
        .unwrap()
    else {
        panic!("expected owned launch reservation")
    };
    let mut stale = current.clone();
    stale.authorization_epoch = 1;
    stale.rooms[0].members[0].exec.policy_revision = 1;
    stale.rooms[0].members[0].exec.enabled = false;
    assert!(apply_configuration_transition(&current, &stale, 1, &registry).is_err());
    assert!(reservation.cancellation.reason().is_none());
    // A batch cannot hide same-revision re-enabling behind a duplicate key.
    let principal = reservation.lease.principal.clone();
    assert!(registry
        .apply_configuration_authorizations(
            1,
            &crate::share::relation_rights::RestrictionSet::default(),
            &[(principal.clone(), 2, false), (principal, 2, true)],
        )
        .is_err());
    assert!(reservation.cancellation.reason().is_none());
    // Epoch 0 remains current; the failed restrictive candidate was never
    // published, and a legitimate retry may still finish its original launch.
    apply_configuration_transition(&current, &current, 0, &registry).unwrap();
    registry
        .commit_start(&reservation.lease, || Ok(()))
        .unwrap();
}
