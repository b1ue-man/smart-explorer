//! Package G1 of plan 2026-10-31 §F (F1–F6): a new connection has no right
//! until it is confirmed, confirming sets exactly the chosen rights,
//! cancelling keeps the connection without rights, rights chosen with the
//! Direct code, the automatic acceptance or a PIN offer apply on connect,
//! the folders of one device bind the actual file access, and stored
//! records of older versions keep their meaning.
use super::*;
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::core::{hmac_proof, presence_payload, public_fingerprint};
use crate::share::relation_rights::{authorization_restrictions, SessionAuthorization};
use crate::share::{
    DirectGrantState, DirectPeerIdentity, DirectReciprocalPeer, DirectRelationMaterial,
    DirectRequestPolicy, PairingOrigin, PeerPresence, ShareExportConfig, ShareIdentity,
    ShareProfiles, SharedRoot, DIRECT_EXPORT_SCOPE,
};

const NOW: i64 = 100;

fn identity() -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[1; 32]);
    let public = secret.public().to_string();
    ShareIdentity {
        device_id: "local-device".into(),
        device_name: "Local".into(),
        direct_lookup_id: "local-lookup".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        iroh_secret: secret,
        direct_secret: [7; 32],
    }
}

/// A request presence carrying this device's Direct code.
fn presence(identity: &ShareIdentity, seed: u8, device: &str) -> PeerPresence {
    let public = iroh::SecretKey::from_bytes(&[seed; 32])
        .public()
        .to_string();
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: identity.direct_lookup_id.clone(),
        device_id: device.into(),
        device_name: format!("Peer {device}"),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        relay_url: "https://relay.invalid".into(),
        candidates: vec!["127.0.0.1:9000".into()],
        expires_at: 200,
        nonce: format!("nonce-{device}"),
        proof: String::new(),
    };
    let payload = presence_payload(
        "direct",
        &presence.relation_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    presence.proof = hmac_proof(&identity.direct_secret(), &payload);
    presence
}

fn profiles_with_exports() -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = vec![
        SharedRoot::new_in_scope("Docs", "/home/u/Docs", DIRECT_EXPORT_SCOPE),
        SharedRoot::new_in_scope("Fotos", "/home/u/Fotos", DIRECT_EXPORT_SCOPE),
    ];
    profiles
}

fn target(profiles: &ShareProfiles, device: &str) -> RightsTarget {
    let grant = profiles.grant_for(device).expect("grant");
    RightsTarget::Direct {
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
    }
}

/// What the file access check gives the session of `device`.
fn session(profiles: &ShareProfiles, device: &str) -> SessionAuthorization {
    let grant = profiles.grant_for(device).expect("grant");
    SessionAuthorization::direct(profiles.default_direct_exports.clone(), grant)
}

fn root_paths(exports: &ShareExportConfig) -> Vec<&str> {
    exports
        .roots
        .iter()
        .map(|root| root.path.as_str())
        .collect()
}

/// A request with the Direct code, accepted by the user (`accept`) or
/// waiting (`Ask`) / accepted automatically (`AutoAccept`).
fn request(profiles: &mut ShareProfiles, device: &str, policy: DirectRequestPolicy, accept: bool) {
    let identity = identity();
    let presence = presence(&identity, device.len() as u8 + 2, device);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &presence, NOW, policy)
        .expect("request recorded");
    if accept {
        let selector = profiles
            .legacy_direct_requests
            .last()
            .unwrap()
            .selector
            .clone();
        profiles
            .decide_legacy_direct_request(&selector, true, NOW)
            .expect("accepted");
    }
}

#[test]
fn abgleich_task_new_connection_has_no_rights_and_waits() {
    let mut profiles = profiles_with_exports();
    request(&mut profiles, "a", DirectRequestPolicy::Ask, true);
    let grant = profiles.grant_for("a").unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted, "connection exists");
    assert!(!grant.write);
    assert!(!grant.exec.grants_any());
    let session = session(&profiles, "a");
    assert!(
        session.exports.roots.is_empty(),
        "no folder before confirmation"
    );
    assert!(!session.exports.shares_connections());
    assert!(!session.may_write);
    let pending = profiles.pending_rights_of(&target(&profiles, "a")).unwrap();
    assert_eq!(pending.origin, RightsOrigin::Request);
    let views = profiles.pending_rights_views();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].rights, ConnectionRights::none());
}

#[test]
fn abgleich_task_confirm_sets_exactly_the_chosen_rights() {
    let mut profiles = profiles_with_exports();
    request(&mut profiles, "a", DirectRequestPolicy::Ask, true);
    let target = target(&profiles, "a");
    let chosen = ConnectionRights {
        folders: FolderChoice::Selected {
            paths: vec!["/home/u/Docs".into()],
            connections: false,
        },
        ..RightsPreset::FilesExec.rights()
    };
    assert!(profiles
        .confirm_connection_rights(&target, &chosen, NOW)
        .unwrap());
    assert!(profiles.pending_rights_of(&target).is_none());
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &target
    else {
        unreachable!()
    };
    assert_eq!(
        profiles.current_direct_rights(device_id, public_key),
        Some(chosen)
    );
    let grant = profiles.grant_for("a").unwrap();
    assert!(grant.write);
    for (capability, expected) in [
        (Capability::ExecCommands, CapabilityDecision::Allow),
        (Capability::ExecTerminal, CapabilityDecision::Allow),
        (Capability::ScreenView, CapabilityDecision::Off),
        (Capability::ScreenControl, CapabilityDecision::Off),
    ] {
        assert_eq!(grant.exec.decision_at(capability, NOW), expected);
    }
    assert_eq!(
        root_paths(&session(&profiles, "a").exports),
        ["/home/u/Docs"]
    );
    // Confirming again with „Nur lesen“ narrows exactly to it.
    let read_only = RightsPreset::ReadOnly.rights();
    profiles
        .confirm_connection_rights(&target, &read_only, NOW + 1)
        .unwrap();
    let grant = profiles.grant_for("a").unwrap();
    assert!(!grant.write);
    assert!(!grant.exec.grants_any());
    assert_eq!(
        root_paths(&session(&profiles, "a").exports),
        ["/home/u/Docs", "/home/u/Fotos"]
    );
}

#[test]
fn abgleich_task_cancel_keeps_the_connection_without_rights() {
    let mut profiles = profiles_with_exports();
    request(&mut profiles, "a", DirectRequestPolicy::Ask, true);
    let target = target(&profiles, "a");
    assert!(profiles.cancel_connection_rights(&target, NOW + 1));
    assert!(profiles.pending_rights.is_empty());
    let grant = profiles.grant_for("a").unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert!(!grant.write && !grant.exec.grants_any());
    assert!(session(&profiles, "a").exports.roots.is_empty());
    let again = profiles.cancel_connection_rights(&target, NOW + 2);
    assert!(!again, "nothing waits");
    // Later, from the device page, the rights can still be confirmed.
    profiles
        .confirm_connection_rights(&target, &RightsPreset::ReadWrite.rights(), NOW)
        .unwrap();
    assert!(session(&profiles, "a").may_write);
}

#[test]
fn abgleich_task_code_preset_gives_exactly_its_rights_on_connect() {
    let mut profiles = profiles_with_exports();
    profiles
        .set_code_rights(Some(RightsPreset::ReadWrite.rights()))
        .unwrap();
    request(&mut profiles, "a", DirectRequestPolicy::Ask, true);
    let first = target(&profiles, "a");
    assert!(profiles.pending_rights_of(&first).is_none());
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &first
    else {
        unreachable!()
    };
    assert_eq!(
        profiles.current_direct_rights(device_id, public_key),
        Some(RightsPreset::ReadWrite.rights())
    );
    // A PIN offer with „Alles“: the publisher installs exactly that.
    let peer = DirectReciprocalPeer::authenticated(
        DirectPeerIdentity::from_secret("pin", "Pin", &iroh::SecretKey::from_bytes(&[9; 32])),
        DirectRelationMaterial::new("remote-lookup", vec![8; 32]).unwrap(),
    )
    .unwrap();
    let all = RightsPreset::All.rights();
    profiles
        .apply_reciprocal_direct_peer_with(
            &peer,
            "c-pin",
            NOW,
            PairingOrigin::UserPairing,
            Some(&all),
        )
        .unwrap();
    // Commands, terminal and screen wait for the Exec-grant journal.
    let pin = target(&profiles, "pin");
    assert!(profiles.pending_rights_of(&pin).is_some());
    let grant = profiles.grant_for("pin").unwrap();
    assert!(grant.write);
    assert_eq!(
        grant.exec.decision_at(Capability::ScreenControl, NOW),
        CapabilityDecision::Off
    );
    assert!(!grant.exec.grants_any());
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    // The connecting side of a PIN pairing chose no rights: it waits.
    let mut connector = profiles_with_exports();
    connector
        .apply_reciprocal_direct_peer(&peer, "c-pin", NOW, PairingOrigin::UserPairing)
        .unwrap();
    let waiting = connector
        .pending_rights_of(&target(&connector, "pin"))
        .unwrap();
    assert_eq!(waiting.origin, RightsOrigin::Pairing);
    assert!(!connector.grant_for("pin").unwrap().write);
}

#[test]
fn abgleich_task_auto_accept_uses_the_chosen_preset_or_waits() {
    let mut without = profiles_with_exports();
    request(&mut without, "a", DirectRequestPolicy::AutoAccept, false);
    let grant = without.grant_for("a").expect("auto-accepted");
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert!(!grant.write, "F6: no preset chosen means no rights");
    assert!(without.pending_rights_of(&target(&without, "a")).is_some());

    let mut with = profiles_with_exports();
    let only_docs = ConnectionRights {
        folders: FolderChoice::Selected {
            paths: vec!["/home/u/Docs".into()],
            connections: false,
        },
        ..RightsPreset::ReadOnly.rights()
    };
    with.set_code_rights(Some(only_docs)).unwrap();
    request(&mut with, "a", DirectRequestPolicy::AutoAccept, false);
    assert!(with.pending_rights.is_empty());
    let session = session(&with, "a");
    assert_eq!(root_paths(&session.exports), ["/home/u/Docs"]);
    assert!(!session.may_write);
}

#[test]
fn abgleich_task_device_folders_bind_the_file_access() {
    let mut profiles = profiles_with_exports();
    request(&mut profiles, "a", DirectRequestPolicy::Ask, true);
    request(&mut profiles, "bb", DirectRequestPolicy::Ask, true);
    let a = target(&profiles, "a");
    let b = target(&profiles, "bb");
    let docs = ConnectionRights {
        folders: FolderChoice::Selected {
            paths: vec!["/home/u/Docs".into()],
            connections: false,
        },
        ..RightsPreset::ReadWrite.rights()
    };
    profiles.confirm_connection_rights(&a, &docs, NOW).unwrap();
    profiles
        .confirm_connection_rights(&b, &RightsPreset::ReadOnly.rights(), NOW)
        .unwrap();
    assert_eq!(
        root_paths(&session(&profiles, "a").exports),
        ["/home/u/Docs"]
    );
    assert_eq!(
        root_paths(&session(&profiles, "bb").exports),
        ["/home/u/Docs", "/home/u/Fotos"]
    );
    // A selection naming another key of the same device id shows nothing.
    let exports = &profiles.default_direct_exports;
    assert!(exports.visible_to("a", "another-key").roots.is_empty());
    // A folder that is no Direct export cannot be selected.
    let missing = ConnectionRights {
        folders: FolderChoice::Selected {
            paths: vec!["/etc".into()],
            connections: false,
        },
        ..ConnectionRights::none()
    };
    assert!(profiles
        .confirm_connection_rights(&a, &missing, NOW)
        .is_err());
    assert_eq!(
        root_paths(&session(&profiles, "a").exports),
        ["/home/u/Docs"]
    );
    // Narrowing a's folders restricts a's sessions, not b's.
    let current = auth_state(&profiles);
    let mut narrowed = profiles.clone();
    narrowed
        .confirm_connection_rights(&a, &RightsPreset::None.rights(), NOW + 1)
        .unwrap();
    let restrictions = authorization_restrictions(&current, &auth_state(&narrowed));
    let grant_a = profiles.grant_for("a").unwrap();
    let grant_b = profiles.grant_for("bb").unwrap();
    assert!(restrictions.affects("direct", "", &grant_a.public_key, &grant_a.node_id));
    assert!(!restrictions.affects("direct", "", &grant_b.public_key, &grant_b.node_id));
}

fn auth_state(profiles: &ShareProfiles) -> crate::share::types::ShareAuthState {
    let identity = identity();
    crate::share::types::ShareAuthState {
        direct_secret: identity.direct_secret().to_vec(),
        identity,
        default_direct_exports: profiles.default_direct_exports.clone(),
        direct_contacts: profiles.direct_contacts.clone(),
        direct_grants: profiles.direct_grants.clone(),
        rooms: profiles.rooms.clone(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }
}

#[test]
fn abgleich_task_older_records_keep_their_rights_unchanged() {
    let stored = r#"{
        "schema_version": 0,
        "default_direct_exports": {"roots": [{"label": "Docs", "path": "/d"}], "include_connections": false},
        "direct_grants": [{
            "device_id": "old", "device_name": "Old", "public_key": "k", "fingerprint": "f",
            "state": "Accepted", "updated_at": 1
        }],
        "rooms": [{"id": "r", "name": "Raum", "room_id": "rid", "auto_join": true,
                   "last_seen": null, "policy": {"members_may_write": true}}]
    }"#;
    let profiles: ShareProfiles = serde_json::from_str(stored).expect("older profile");
    let grant = &profiles.direct_grants[0];
    assert!(grant.write, "grants before V5 keep writing");
    assert!(profiles.pending_rights.is_empty());
    assert!(profiles.code_rights.is_none());
    assert!(profiles.default_direct_exports.device_folders.is_empty());
    let session = session(&profiles, "old");
    assert_eq!(
        root_paths(&session.exports),
        ["/d"],
        "every export as before"
    );
    assert!(session.may_write);
    assert!(profiles.rooms[0].policy.member_access.is_empty());
    assert_eq!(
        profiles.current_direct_rights("old", "k").unwrap().folders,
        FolderChoice::All
    );
    // New fields stay out of files that do not use them.
    let written = serde_json::to_string(&ShareProfiles::default()).unwrap();
    for field in [
        "pending_rights",
        "code_rights",
        "device_folders",
        "member_access",
    ] {
        assert!(!written.contains(field), "{field} in {written}");
    }
}

#[test]
fn abgleich_task_presets_map_to_the_menu_rights() {
    let none = RightsPreset::None.rights();
    assert!(!none.grants_anything());
    assert_eq!(RightsPreset::ReadOnly.rights().folders, FolderChoice::All);
    assert!(!RightsPreset::ReadOnly.rights().write);
    assert!(RightsPreset::ReadWrite.rights().write);
    let exec = RightsPreset::FilesExec.rights();
    assert_eq!(
        (exec.commands, exec.terminal, exec.screen_control),
        (
            CapabilityDecision::Allow,
            CapabilityDecision::Allow,
            CapabilityDecision::Off
        )
    );
    let all = RightsPreset::All.rights();
    assert_eq!(all.screen_view, CapabilityDecision::Allow);
    assert_eq!(all.screen_control, CapabilityDecision::Allow);
    for preset in RightsPreset::ALL {
        assert_eq!(RightsPreset::from_key(preset.key()), Some(preset));
        assert_eq!(preset.rights().preset(), Some(preset));
        let encoded = serde_json::to_string(&preset).unwrap();
        assert_eq!(encoded, format!("\"{}\"", preset.key()));
    }
    let change = all.access_change();
    for capability in MENU_CAPABILITIES {
        assert!(change.capabilities.contains_key(&capability));
    }
    assert!(!change.capabilities.contains_key(&Capability::LoginAccess));
    assert_eq!(preset_views().len(), 5);
}

#[test]
fn abgleich_task_room_rights_wait_then_apply_to_members() {
    let mut profiles = profiles_with_exports();
    profiles.rooms.push(crate::share::RoomProfile {
        id: "room-1".into(),
        name: "Team".into(),
        room_id: "rid".into(),
        auto_join: true,
        last_seen: None,
        status: Default::default(),
        members: Vec::new(),
        exports: Default::default(),
        policy: crate::share::RoomPolicy::new_room(),
    });
    profiles.mark_room_rights_pending("room-1", RightsOrigin::RoomCreated, NOW);
    let room = RightsTarget::Room {
        profile_id: "room-1".into(),
    };
    assert!(profiles.pending_rights_of(&room).is_some());
    assert_eq!(
        profiles.current_room_rights("room-1").unwrap().folders,
        FolderChoice::None
    );
    profiles
        .confirm_connection_rights(&room, &RightsPreset::FilesExec.rights(), NOW)
        .unwrap();
    assert!(profiles.pending_rights_of(&room).is_none());
    let room_profile = &mut profiles.rooms[0];
    assert!(room_profile.policy.members_may_write);
    assert_eq!(room_profile.exports.roots.len(), 2);
    let identity = identity();
    let mut member = presence(&identity, 11, "member");
    member.kind = "room".into();
    member.relation_id = "rid".into();
    room_profile.upsert_member_from_presence(member, NOW);
    // Recorded for the Exec-grant journal; the grant stays as it is.
    assert!(room_profile.apply_member_access("member", NOW));
    assert_eq!(
        room_profile.members[0]
            .exec
            .decision_at(Capability::ExecCommands, NOW),
        CapabilityDecision::Off
    );
    assert_eq!(room_profile.policy.member_follow_ups.len(), 1);
    assert!(
        !room_profile.apply_member_access("member", NOW),
        "recorded once"
    );
}
