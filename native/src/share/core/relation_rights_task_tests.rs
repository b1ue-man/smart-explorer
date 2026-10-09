//! Acceptance of scoped reductions and preservation of existing authority.
use super::*;
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use crate::share::removed_direct_peers::PairingOrigin;
use crate::share::types::{DirectGrantState, PeerPresence};
use crate::share::{ExportAccess, RoomPolicy, SharedRoot};

fn peer(seed: u8, device: &str) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(device, device, &iroh::SecretKey::from_bytes(&[seed; 32]))
}

fn grant(peer: &DirectPeerIdentity) -> DirectGrant {
    DirectGrant {
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
        write: true,
    }
}

fn state() -> ShareAuthState {
    let key = iroh::SecretKey::from_bytes(&[3; 32]);
    let local = peer(3, "local");
    ShareAuthState {
        identity: crate::share::ShareIdentity {
            device_id: local.device_id,
            device_name: local.device_name,
            direct_lookup_id: "local-lookup".into(),
            public_key: local.public_key,
            fingerprint: local.fingerprint,
            node_id: local.node_id,
            iroh_secret: key,
            direct_secret: [7; 32],
        },
        direct_secret: vec![7; 32],
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![grant(&peer(4, "a")), grant(&peer(5, "b"))],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    }
}

fn presence(peer: &DirectPeerIdentity, room_id: &str) -> PeerPresence {
    PeerPresence {
        kind: "room".into(),
        relation_id: room_id.into(),
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
        relay_url: String::new(),
        candidates: Vec::new(),
        expires_at: 100,
        nonce: "verified.ps1.signature".into(),
        proof: String::new(),
    }
}

fn room(room_id: &str) -> RoomProfile {
    RoomProfile {
        id: format!("profile-{room_id}"),
        name: room_id.into(),
        room_id: room_id.into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: Vec::new(),
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    }
}

#[test]
fn review_task_restrictions_ignore_runtime_and_extensions_but_revoke_exact_key() {
    let before = state();
    let mut after = before.clone();
    after.direct_grants[0].updated_at = 9;
    after.direct_grants[0].device_name = "Renamed".into();
    after
        .default_direct_exports
        .roots
        .push(SharedRoot::new("Docs", "sftp://u@nas:22/docs"));
    assert!(authorization_restrictions(&before, &after).is_empty());
    after.direct_grants[0].write = false;
    let restrictions = authorization_restrictions(&before, &after);
    let a = peer(4, "different-device-alias");
    let b = peer(5, "b");
    assert!(restrictions.affects("direct", "either-lookup", &a.public_key, &a.node_id));
    assert!(!restrictions.affects("direct", "either-lookup", &b.public_key, &b.node_id));
    assert!(!restrictions.affects("room", "r", &a.public_key, &a.node_id));
    assert!(restrictions.everything_reason().is_none());
}

#[test]
fn review_task_export_reduction_preserves_backend_identity_and_scopes_room() {
    let mut before = state();
    let mut first = room("r1");
    first
        .exports
        .roots
        .push(SharedRoot::new("Docs", "sftp://u@one:22/docs").with_access(ExportAccess::ReadWrite));
    before.rooms.push(first);
    before.rooms.push(room("r2"));
    let mut after = before.clone();
    after.rooms[0].exports.roots[0].path = "sftp://u@two:22/docs".into();
    let restrictions = authorization_restrictions(&before, &after);
    assert!(restrictions.affects("room", "r1", "any-key", "any-node"));
    assert!(!restrictions.affects("room", "r2", "any-key", "any-node"));
    assert!(!restrictions.affects("direct", "r1", "any-key", "any-node"));
    assert_eq!(
        restrictions.items()[0].reason,
        RestrictionReason::ExportsNarrowed
    );
}

#[test]
fn review_task_room_block_rejects_key_alias_and_new_members_wait_without_exec() {
    let mut room = room("r");
    let a = peer(4, "a");
    room.upsert_member_from_presence(presence(&a, "r"), 1);
    room.members[0].exec.set_runtime_enabled(true, 1).unwrap();
    let revision = room.members[0].exec.policy_revision;
    assert!(room.set_member_blocked("a", true, 2));
    assert!(room.policy.confirm_new_members);
    assert!(!room.members[0].is_admitted());
    assert!(!room.members[0].exec.enabled);
    assert!(room.members[0].exec.policy_revision > revision);
    let alias = peer(4, "alias");
    room.upsert_member_from_presence(presence(&alias, "r"), 3);
    assert_eq!(room.members.len(), 1);
    room.upsert_member_from_presence(presence(&peer(5, "b"), "r"), 4);
    assert!(!room.members[1].is_admitted());
    assert!(!room.members[1].exec.enabled);
    assert!(room.admit_member("b"));
    assert!(room.members[1].is_admitted());
    assert!(!room.members[1].exec.enabled);
}

#[test]
fn review_task_one_way_pairing_and_repair_never_create_share_back_grant() {
    let mut profiles = ShareProfiles::default();
    let peer = DirectReciprocalPeer::authenticated(
        peer(4, "a"),
        DirectRelationMaterial::new("remote-lookup", vec![8; 32]).unwrap(),
    )
    .unwrap();
    profiles
        .apply_reciprocal_direct_peer(&peer, "c", 1, PairingOrigin::UserPairingOneWay)
        .unwrap();
    assert!(!profiles.direct_contacts[0].relation.share_back);
    assert!(profiles.direct_grants.is_empty());
    profiles
        .apply_reciprocal_direct_peer(&peer, "unused", 2, PairingOrigin::AutomaticRepair)
        .unwrap();
    assert!(profiles.direct_grants.is_empty());
    profiles.set_contact_share_back("c", true, 3).unwrap();
    assert_eq!(profiles.direct_grants.len(), 1);
    // A new Direct grant may write (default since 2026-10-09).
    assert!(profiles.direct_grants[0].write);
    profiles.direct_grants[0].state = DirectGrantState::Reconfirm;
    profiles
        .apply_reciprocal_direct_peer(&peer, "unused", 4, PairingOrigin::AutomaticRepair)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Reconfirm);
    profiles
        .apply_reciprocal_direct_peer(&peer, "unused", 5, PairingOrigin::UserPairing)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Accepted);
    assert!(!profiles.direct_grants[0].exec.enabled);
    assert!(profiles.direct_contacts[0].relation.share_back);
    profiles
        .apply_reciprocal_direct_peer(&peer, "unused", 6, PairingOrigin::UserPairingOneWay)
        .unwrap();
    assert!(!profiles.direct_contacts[0].relation.share_back);
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Accepted);
}

#[test]
fn review_task_withdrawal_denies_device_alias_until_deliberate_readmission() {
    let a = peer(4, "a");
    let alias = peer(4, "alias");
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants = vec![grant(&a), grant(&alias), grant(&peer(5, "b"))];
    profiles.direct_grants[1]
        .exec
        .set_runtime_enabled(true, 1)
        .unwrap();
    profiles.withdraw_direct_key(&a, 2);
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    assert_eq!(profiles.direct_grants[1].state, DirectGrantState::Ignored);
    assert!(!profiles.direct_grants[1].exec.enabled);
    assert_eq!(profiles.direct_grants[2].state, DirectGrantState::Accepted);
    assert!(profiles.direct_auto_accept_denied("lookup", &alias));
    profiles.allow_direct_grant_again("a", 3).unwrap();
    assert_eq!(profiles.direct_grants[1].state, DirectGrantState::Accepted);
    assert!(!profiles.direct_auto_accept_denied("lookup", &alias));
    assert!(!profiles.direct_grants[1].exec.enabled);
}
