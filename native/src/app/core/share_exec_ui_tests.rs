use super::{activation_ready, exec_device_views, exec_warning};
use crate::share::{
    DirectContact, DirectGrant, DirectGrantState, ExecGrant, ExecProviderStatus, RoomMember,
    RoomProfile, ShareProfiles, ShareStatus,
};

fn contact() -> DirectContact {
    DirectContact {
        id: "contact-a".into(),
        display_name: "Peer A".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: "fp-a".into(),
        expected_node_id: "key-a".into(),
        remote_device_id: Some("device-a".into()),
        remote_public_key: Some("key-a".into()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Waiting,
        last_error: None,
        presence: None,
        access_state: crate::share::DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
    }
}

fn grant() -> DirectGrant {
    DirectGrant {
        device_id: "device-a".into(),
        device_name: "Peer A".into(),
        public_key: "key-a".into(),
        fingerprint: "fp-a".into(),
        node_id: "key-a".into(),
        state: DirectGrantState::Accepted,
        updated_at: 10,
        exec: ExecGrant {
            enabled: true,
            policy_revision: 7,
            changed_at: 9,
            source_request_id: None,
            source_decision_revision: None,
        },
    }
}

fn room() -> RoomProfile {
    RoomProfile {
        id: "room-profile".into(),
        name: "Ops".into(),
        room_id: "room-wire".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![RoomMember {
            device_id: "device-b".into(),
            device_name: "Peer B".into(),
            fingerprint: "fp-b".into(),
            public_key: "key-b".into(),
            node_id: "key-b".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: ShareStatus::Waiting,
            blocked: false,
            exec: ExecGrant::default(),
            presence: None,
        }],
        exports: Default::default(),
    }
}

#[test]
fn projects_every_direct_grant_and_room_member_with_exact_state() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(contact());
    profiles.direct_grants.push(grant());
    profiles.rooms.push(room());

    let views = exec_device_views(&profiles);
    assert_eq!(views.len(), 2);
    let direct = views
        .iter()
        .find(|view| view.device_id == "device-a")
        .unwrap();
    assert!(direct.enabled);
    assert_eq!(direct.policy_revision, 7);
    assert_eq!(
        direct.target,
        crate::share::ExecGrantTarget::Direct {
            device_id: "device-a".into(),
            public_key: "key-a".into(),
            fingerprint: "fp-a".into(),
            node_id: "key-a".into(),
        }
    );
    let room = views
        .iter()
        .find(|view| view.device_id == "device-b")
        .unwrap();
    assert!(!room.enabled);
    assert!(room.base_authorized);
    assert_eq!(
        room.target,
        crate::share::ExecGrantTarget::RoomMember {
            room_id: "room-wire".into(),
            device_id: "device-b".into(),
            public_key: "key-b".into(),
            fingerprint: "fp-b".into(),
            node_id: "key-b".into(),
        }
    );
}

#[test]
fn direct_target_is_built_only_from_the_grants_exact_identity_pins() {
    let mut profiles = ShareProfiles::default();
    let mut mismatched = contact();
    mismatched.expected_fingerprint = "different-fingerprint".into();
    profiles.direct_contacts.push(mismatched);
    profiles.direct_grants.push(grant());

    let views = exec_device_views(&profiles);
    assert_eq!(views.len(), 1);
    assert_eq!(
        views[0].target,
        crate::share::ExecGrantTarget::Direct {
            device_id: "device-a".into(),
            public_key: "key-a".into(),
            fingerprint: "fp-a".into(),
            node_id: "key-a".into(),
        }
    );
    assert!(views[0].enabled);
}

#[test]
fn warning_names_full_user_execution_and_elevated_shell() {
    let root = ExecProviderStatus {
        available: true,
        provider: "systemd".into(),
        detail: String::new(),
        elevated: true,
        user_label: "root".into(),
    };
    let root_warning = exec_warning(&root);
    assert_eq!(root_warning.full_access, "FULL root CODE EXECUTION");
    assert_eq!(root_warning.elevated, Some("REMOTE ROOT SHELL"));

    let admin = ExecProviderStatus {
        user_label: "Alice".into(),
        ..root
    };
    let admin_warning = exec_warning(&admin);
    assert_eq!(admin_warning.full_access, "FULL Alice CODE EXECUTION");
    assert_eq!(admin_warning.elevated, Some("REMOTE ADMINISTRATOR SHELL"));
}

#[test]
fn activation_requires_explicit_confirmation_provider_and_base_grant() {
    let mut provider = ExecProviderStatus {
        available: true,
        provider: "containment".into(),
        detail: String::new(),
        elevated: false,
        user_label: "alice".into(),
    };
    assert!(!activation_ready(false, &provider, true));
    assert!(!activation_ready(true, &provider, false));
    provider.available = false;
    assert!(!activation_ready(true, &provider, true));
    provider.available = true;
    assert!(activation_ready(true, &provider, true));
}

#[test]
fn inactive_base_authorization_remains_visible_but_cannot_be_enabled() {
    let mut profiles = ShareProfiles::default();
    let mut ignored = grant();
    ignored.state = DirectGrantState::Ignored;
    profiles.direct_contacts.push(contact());
    profiles.direct_grants.push(ignored);
    let mut inactive_room = room();
    inactive_room.auto_join = false;
    profiles.rooms.push(inactive_room);

    let views = exec_device_views(&profiles);
    assert_eq!(views.len(), 2);
    assert!(views.iter().all(|view| !view.base_authorized));
}
