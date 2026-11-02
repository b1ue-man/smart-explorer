//! Contact, room and export-order settings shared by CLI, desktop and Android.
use crate::share::{
    DirectContact, DirectGrant, DirectGrantState, ExecGrant, RoomMember, RoomProfile,
    ShareProfiles, ShareStatus, SharedRoot,
};

fn contact() -> DirectContact {
    DirectContact {
        id: "contact-a".into(),
        display_name: "Peer A".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: "fp-a".into(),
        expected_node_id: "node-a".into(),
        remote_device_id: Some("device-a".into()),
        remote_public_key: Some("key-a".into()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Available,
        last_error: None,
        presence: None,
        access_state: crate::share::DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn grant() -> DirectGrant {
    DirectGrant {
        device_id: "device-a".into(),
        device_name: "Peer A".into(),
        public_key: "key-a".into(),
        fingerprint: "fp-a".into(),
        node_id: "node-a".into(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant {
            enabled: true,
            policy_revision: 3,
            ..ExecGrant::default()
        },
        write: true,
    }
}

fn room() -> RoomProfile {
    RoomProfile {
        id: "room-profile".into(),
        name: "Team".into(),
        room_id: "room-wire".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Available,
        members: vec![RoomMember {
            device_id: "device-b".into(),
            device_name: "Peer B".into(),
            fingerprint: "fp-b".into(),
            public_key: "key-b".into(),
            node_id: "node-b".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: ShareStatus::Available,
            blocked: false,
            exec: ExecGrant::default(),
            presence: None,
            relation: Default::default(),
        }],
        exports: Default::default(),
        policy: crate::share::RoomPolicy::new_room(),
    }
}

#[test]
fn luecken_task_contact_options_change_only_the_named_values() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(contact());
    assert!(!profiles
        .set_contact_options("contact-a", Some(true), None)
        .unwrap());
    assert!(profiles
        .set_contact_options("contact-a", Some(false), Some(true))
        .unwrap());
    assert!(!profiles.direct_contacts[0].auto_connect);
    assert!(profiles.direct_contacts[0].auto_open);
    assert!(profiles
        .set_contact_options("missing", Some(true), None)
        .is_err());
}

#[test]
fn luecken_task_contact_trust_reset_forgets_the_pin_and_withdraws_its_grant() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(contact());
    profiles.direct_grants.push(grant());
    assert!(profiles.reset_contact_trust("contact-a", 50).unwrap());
    let contact = &profiles.direct_contacts[0];
    assert!(contact.remote_device_id.is_none() && contact.remote_public_key.is_none());
    assert_eq!(contact.status, ShareStatus::Waiting);
    let grant = &profiles.direct_grants[0];
    assert_eq!(grant.state, DirectGrantState::Ignored);
    assert!(!grant.exec.enabled);
    assert_eq!(grant.exec.policy_revision, 4);
    assert!(!profiles.reset_contact_trust("contact-a", 51).unwrap());
    assert!(profiles.reset_contact_trust("missing", 52).is_err());
}

#[test]
fn luecken_task_room_rename_join_and_member_trust() {
    let mut profiles = ShareProfiles::default();
    profiles.rooms.push(room());
    assert!(profiles
        .rename_room("room-profile", "  Werkstatt ")
        .unwrap());
    assert_eq!(profiles.rooms[0].name, "Werkstatt");
    assert!(profiles.rename_room("room-profile", "").unwrap());
    assert_eq!(profiles.rooms[0].name, "Raum");
    assert!(profiles.rename_room("room-profile", "a\u{7}b").is_err());
    assert!(profiles.rename_room("missing", "x").is_err());

    assert!(profiles.set_room_auto_join("room-profile", false).unwrap());
    assert_eq!(profiles.rooms[0].status, ShareStatus::Offline);
    assert!(!profiles.set_room_auto_join("room-profile", false).unwrap());
    assert!(profiles.set_room_auto_join("room-profile", true).unwrap());

    assert!(profiles
        .reset_member_trust("room-profile", "device-b")
        .unwrap());
    assert_eq!(profiles.rooms[0].members[0].status, ShareStatus::Waiting);
    assert!(!profiles
        .reset_member_trust("room-profile", "device-b")
        .unwrap());
    assert!(profiles
        .reset_member_trust("room-profile", "nobody")
        .is_err());
}

#[test]
fn luecken_task_export_order_moves_one_root_and_stops_at_the_ends() {
    let mut profiles = ShareProfiles::default();
    for name in ["a", "b", "c"] {
        profiles
            .default_direct_exports
            .roots
            .push(SharedRoot::new_in_scope(
                name,
                format!("/srv/{name}"),
                "direct",
            ));
    }
    let order = |profiles: &ShareProfiles| {
        profiles
            .default_direct_exports
            .roots
            .iter()
            .map(|root| root.label.clone())
            .collect::<Vec<_>>()
    };
    assert!(profiles.move_export_root("direct", "/srv/c", -1).unwrap());
    assert_eq!(order(&profiles), ["a", "c", "b"]);
    assert!(!profiles.move_export_root("direct", "/srv/a", -1).unwrap());
    assert!(profiles.move_export_root("direct", "/srv/a", 5).unwrap());
    assert_eq!(order(&profiles), ["c", "b", "a"]);
    assert!(profiles.move_export_root("direct", "/srv/x", 1).is_err());
    assert!(profiles.move_export_root("no-room", "/srv/a", 1).is_err());
}
