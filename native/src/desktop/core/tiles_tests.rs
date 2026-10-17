//! DSK-M1: tile projection (C4.4–C4.6) with the online rule and the status
//! codes of the Android bridge (`share::device_view`).
use std::collections::BTreeMap;

use super::tiles::{
    device_key, project, EmptyAction, ShareState, TileCode, TileInput, TileModel, TileOpen,
    VaultTile, UNKNOWN_TEXT,
};
use crate::share::device_view;
use crate::share::{
    DirectAccessState, DirectContact, ExecGrant, PeerOpenTarget, RoomMember, RoomPolicy,
    RoomProfile, ShareProfiles, ShareStatus,
};

const NOW: i64 = 1_800_000_000;

fn contact(id: &str, status: ShareStatus) -> DirectContact {
    DirectContact {
        id: id.into(),
        display_name: format!("Gerät {id}"),
        lookup_id: format!("lookup-{id}"),
        expected_fingerprint: format!("fp-{id}"),
        expected_node_id: format!("node-{id}"),
        remote_device_id: Some(format!("device-{id}")),
        remote_public_key: Some(format!("key-{id}")),
        auto_connect: true,
        auto_open: false,
        last_seen: Some(NOW - 600),
        status,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: vec!["192.0.2.7:4433".into()],
        lan_seen_at: Some(NOW - 10),
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn member(device_id: &str, status: ShareStatus) -> RoomMember {
    RoomMember {
        device_id: device_id.into(),
        device_name: format!("Mitglied {device_id}"),
        fingerprint: format!("fp-{device_id}"),
        public_key: format!("key-{device_id}"),
        node_id: format!("node-{device_id}"),
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: None,
        status,
        blocked: false,
        exec: ExecGrant::default(),
        presence: None,
        relation: Default::default(),
    }
}

fn room(id: &str, members: Vec<RoomMember>) -> RoomProfile {
    RoomProfile {
        id: id.into(),
        name: format!("Raum {id}"),
        room_id: format!("wire-{id}"),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Available,
        members,
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    }
}

fn every_status() -> [ShareStatus; 10] {
    [
        ShareStatus::Offline,
        ShareStatus::Waiting,
        ShareStatus::WaitingForAccess,
        ShareStatus::Available,
        ShareStatus::Connecting,
        ShareStatus::Connected,
        ShareStatus::ConnectedDirect,
        ShareStatus::ConnectedRelay,
        ShareStatus::Failed("Zeitüberschreitung".into()),
        ShareStatus::IdentityConflict,
    ]
}

/// Ten Direct devices (one per status) and two rooms.
fn profiles() -> ShareProfiles {
    let direct_contacts = every_status()
        .into_iter()
        .enumerate()
        .map(|(index, status)| contact(&format!("c{index}"), status))
        .collect();
    let members = vec![
        member("d1", ShareStatus::ConnectedRelay),
        member("d2", ShareStatus::IdentityConflict),
    ];
    ShareProfiles {
        direct_contacts,
        rooms: vec![room("raum-1", members), room("raum-2", Vec::new())],
        ..ShareProfiles::default()
    }
}

fn model(profiles: &ShareProfiles, share_state: ShareState) -> TileModel {
    let mut paths = BTreeMap::new();
    paths.insert(
        "direct~c3".to_string(),
        "/home/nutzer/SmartExplorer/Geräte/Gerät c3".to_string(),
    );
    project(&TileInput {
        profiles: Ok(profiles),
        share_state: &share_state,
        now_secs: NOW,
        device_paths: &paths,
        vaults: None,
    })
}

fn profiles_only() -> ShareState {
    ShareState::Profiles {
        reason: "Hintergrunddienst läuft nicht".into(),
    }
}

#[test]
fn desktop_task_tiles_every_status_code() {
    let profiles = profiles();
    let live = model(&profiles, ShareState::Live);
    assert_eq!(live.devices.len(), 10);
    for (tile, contact) in live.devices.iter().zip(&profiles.direct_contacts) {
        let status = &contact.status;
        assert_eq!(tile.code, TileCode::from_status(status));
        assert_eq!(tile.code.as_str(), device_view::status_code(status));
        assert_eq!(tile.status_text, status.label());
        assert_eq!(tile.online, device_view::online(contact, NOW));
        assert_eq!(tile.lan, device_view::seen_on_lan(contact, NOW));
        assert_eq!(tile.write, device_view::granted_write(&profiles, contact));
        assert_eq!(tile.key, format!("direct~{}", contact.id));
        assert_eq!(tile.location, format!("share://direct/{}/", contact.id));
        assert_eq!(tile.device_id, contact.remote_device_id);
        // C4.5: waiting for access and a changed identity open the device page.
        let page = format!("share-device:{}", contact.id);
        let expected = match status {
            ShareStatus::WaitingForAccess | ShareStatus::IdentityConflict => TileOpen::Page(page),
            _ => TileOpen::Location(tile.location.clone()),
        };
        assert_eq!(tile.open, expected, "{status:?}");
    }
    // Ten status codes plus `unknown`, each spelled as on the bridge.
    let unknown = model(&profiles, profiles_only());
    let mut codes: Vec<&str> = live.devices.iter().map(|tile| tile.code.as_str()).collect();
    codes.push(unknown.devices[0].code.as_str());
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 11);
    assert!(codes.contains(&"unknown"));
    // The S2 folder of a mounted device tree is attached by key.
    assert_eq!(
        live.devices[3].folder_path.as_deref(),
        Some("/home/nutzer/SmartExplorer/Geräte/Gerät c3")
    );
    assert!(live.devices[0].folder_path.is_none());

    // Rooms are folder groups with their members (C4.2).
    assert_eq!(live.rooms.len(), 2);
    let first = &live.rooms[0];
    assert_eq!(
        (first.room_id.as_str(), first.name.as_str()),
        ("raum-1", "Raum raum-1")
    );
    assert_eq!(first.page, "share-room:raum-1");
    assert_eq!(
        first.members[0].target,
        PeerOpenTarget::RoomDevice {
            room_id: "raum-1".into(),
            device_id: "d1".into()
        }
    );
    assert_eq!(first.members[0].key, "room~raum-1~d1");
    assert_eq!(
        first.members[0].open,
        TileOpen::Location("share://room/raum-1/d1/".into())
    );
    assert_eq!(first.members[0].device_id.as_deref(), Some("d1"));
    assert!(first.members[0].online && !first.members[0].lan);
    // An identity conflict never opens files.
    assert_eq!(first.members[1].code, TileCode::IdentityConflict);
    assert_eq!(
        first.members[1].open,
        TileOpen::Page("share-room:raum-1".into())
    );
    assert!(live.rooms[1].members.is_empty());
    assert!(live.empty_actions.is_empty());
    assert!(live.vaults.is_none());

    // Keys escape every byte outside [A-Za-z0-9._-] as %XX (V4).
    assert_eq!(
        device_key(&PeerOpenTarget::Direct {
            contact_id: "a b/ü~".into()
        }),
        "direct~a%20b%2F%C3%BC%7E"
    );
    assert_eq!(
        device_key(&PeerOpenTarget::RoomDevice {
            room_id: "r.1_x-y".into(),
            device_id: "D:2".into()
        }),
        "room~r.1_x-y~D%3A2"
    );
}

#[test]
fn desktop_task_tiles_profiles_only_unknown() {
    let profiles = profiles();
    let model = model(&profiles, profiles_only());
    assert_eq!(model.share_state, profiles_only());
    let all = model
        .devices
        .iter()
        .chain(model.rooms.iter().flat_map(|room| room.members.iter()));
    for tile in all {
        let conflict = tile.status_text == ShareStatus::IdentityConflict.label();
        if conflict {
            // A stored identity conflict stays visible and opens the page.
            assert_eq!(tile.code, TileCode::IdentityConflict);
            assert!(matches!(tile.open, TileOpen::Page(_)));
        } else {
            // Without the service nothing is known; the window connects.
            assert_eq!(tile.code, TileCode::Unknown, "{}", tile.name);
            assert_eq!(tile.status_text, UNKNOWN_TEXT);
            assert_eq!(tile.open, TileOpen::Location(tile.location.clone()));
        }
        assert!(!tile.online && !tile.lan, "{}", tile.name);
    }
    // Profiles that cannot be read give one error line, no tiles (C4.6).
    let paths = BTreeMap::new();
    let state = profiles_only();
    let broken = project(&TileInput {
        profiles: Err("share-profiles.json: unerwartetes Ende"),
        share_state: &state,
        now_secs: NOW,
        device_paths: &paths,
        vaults: None,
    });
    assert_eq!(
        broken.profiles_error.as_deref(),
        Some("share-profiles.json: unerwartetes Ende")
    );
    assert!(broken.devices.is_empty() && broken.rooms.is_empty());
    assert!(broken.empty_actions.is_empty());
}

#[test]
fn desktop_task_tiles_empty_profiles() {
    let empty = ShareProfiles::default();
    for state in [ShareState::Live, profiles_only()] {
        let model = model(&empty, state);
        assert!(model.devices.is_empty() && model.rooms.is_empty());
        assert_eq!(
            model.empty_actions,
            [EmptyAction::AddDevice, EmptyAction::JoinRoom]
        );
    }
    assert_eq!(
        [EmptyAction::AddDevice, EmptyAction::JoinRoom].map(EmptyAction::page),
        ["share-devices", "share-rooms"]
    );
    assert_eq!(
        [EmptyAction::AddDevice, EmptyAction::JoinRoom].map(EmptyAction::label),
        ["Gerät hinzufügen", "Raum beitreten"]
    );
    // A room alone ends the empty state.
    let with_room = ShareProfiles {
        rooms: vec![room("raum-1", Vec::new())],
        ..ShareProfiles::default()
    };
    assert!(model(&with_room, ShareState::Live).empty_actions.is_empty());
    // Vault tiles come only from S3 and keep site, label and state.
    let vaults = [VaultTile {
        site: "3f2a9c1b".into(),
        label: "Steuer".into(),
        state: "locked".into(),
    }];
    let paths = BTreeMap::new();
    let state = ShareState::Live;
    let with_vaults = project(&TileInput {
        profiles: Ok(&empty),
        share_state: &state,
        now_secs: NOW,
        device_paths: &paths,
        vaults: Some(&vaults),
    });
    assert_eq!(with_vaults.vaults.as_deref(), Some(&vaults[..]));
    assert_eq!(vaults[0].location(), "vault://3f2a9c1b/");
}
