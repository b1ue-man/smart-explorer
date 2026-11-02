use crate::share::{
    DirectGrant, DirectGrantState, DirectPeerIdentity, ExportAccess, ShareExportConfig,
    ShareProfiles,
};

fn peer() -> DirectPeerIdentity {
    DirectPeerIdentity {
        device_id: "d1".into(),
        device_name: "Peer".into(),
        public_key: "key1".into(),
        node_id: "node1".into(),
        fingerprint: "fp1".into(),
    }
}
fn grant() -> DirectGrant {
    let peer = peer();
    DirectGrant {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
        node_id: peer.node_id,
        fingerprint: peer.fingerprint,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
        write: false,
    }
}

#[test]
fn review_task_fc1_write_edits_do_not_readmit_withdrawn_or_replaced_keys() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(grant());
    assert!(profiles.set_direct_peer_write(&peer(), true, 2).unwrap());
    profiles.withdraw_direct_key(&peer(), 3);
    assert!(profiles.set_direct_peer_write(&peer(), true, 4).is_err());
    profiles.set_direct_peer_write(&peer(), false, 5).unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    assert!(!profiles.direct_grants[0].write);
    profiles.direct_grants[0].state = DirectGrantState::Accepted;
    profiles.record_removed_direct_peer(&peer(), 6);
    assert!(profiles.set_direct_peer_write(&peer(), true, 7).is_err());
    assert_eq!(profiles.removed_direct_peers.len(), 1);
    profiles.direct_grants[0].public_key = "replacement".into();
    assert!(profiles.set_direct_peer_write(&peer(), false, 8).is_err());
    assert_eq!(profiles.direct_grants[0].public_key, "replacement");
}

#[test]
fn review_task_fc1_same_relative_path_on_two_saved_remotes_is_separate() {
    let mut config = ShareExportConfig::default();
    config
        .set_connection_access("sftp://u@a:22/docs", Some(ExportAccess::ReadOnly))
        .unwrap();
    config
        .set_connection_access("sftp://u@b:22/docs", Some(ExportAccess::ReadWrite))
        .unwrap();
    config
        .set_connection_access("sftp://u@a:22/docs", None)
        .unwrap();
    assert_eq!(config.connection_access("sftp://u@a:22/docs"), None);
    assert_eq!(
        config.connection_access("sftp://u@b:22/docs"),
        Some(ExportAccess::ReadWrite)
    );
    assert!(config
        .set_root_access("/missing", ExportAccess::ReadWrite, None)
        .is_err());
    assert!(config.roots.is_empty());
}

#[test]
fn review_task_fc1_room_policy_does_not_admit_pending_or_blocked_members() {
    let mut profiles = ShareProfiles::default();
    let member: crate::share::RoomMember = serde_json::from_value(serde_json::json!({
        "device_id":"d", "device_name":"Peer", "fingerprint":"fp", "public_key":"key",
        "node_id":"node", "candidates":[], "last_seen":null, "blocked":true,
        "relation":{"admission":"Pending"}
    }))
    .unwrap();
    profiles.rooms.push(crate::share::RoomProfile {
        id: "p".into(),
        room_id: "r".into(),
        name: "Room".into(),
        auto_join: true,
        last_seen: None,
        status: Default::default(),
        members: vec![member],
        exports: Default::default(),
        policy: crate::share::RoomPolicy::new_room(),
    });
    profiles
        .set_room_policy("p", Some(true), Some(false))
        .unwrap();
    assert!(profiles.rooms[0].members[0].blocked);
    assert_eq!(
        profiles.rooms[0].members[0].relation.admission,
        crate::share::RoomMemberAdmission::Pending
    );
    assert!(!profiles.rooms[0].members[0].exec.enabled);
    assert!(profiles.rooms[0].exports.roots.is_empty());
}
