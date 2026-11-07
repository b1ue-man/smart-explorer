//! Findings W6, W7, W8 (core part), W9 and G8 of the second review
//! 2026-10-31 (`docs/plaene/2026-10-31-sync-abgleich-schnell/review2.md`):
//! a readmitted device starts without rights, a narrower folder selection
//! ends work in the lost folders also with other changes in the same step,
//! the confirmation names exactly the Exec grants it changes, profiles with
//! rights records are written so that older versions refuse them, and the
//! test store joins rooms like the production store.
use super::changed_exec_grants;
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, RightsOrigin, RightsPreset, RightsTarget,
};
use crate::share::core::public_fingerprint;
use crate::share::direct_protocol::{DirectDecisionKind, DirectRequestId, SignedDirectRequest};
use crate::share::profile_persistence::ProfilePersistence;
use crate::share::profiles::ProfileRevision;
use crate::share::relation_rights::{authorization_restrictions, SessionAuthorization};
use crate::share::{
    DirectGrantState, DirectPeerIdentity, ShareIdentity, ShareProfiles, SharedRoot,
    DIRECT_EXPORT_SCOPE,
};

// The same fixture as `direct_ledger_tests.rs`, compiled here once more.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "direct_ledger_fixture.rs"]
mod ledger;

const NOW: i64 = 100;

fn profiles_with_exports() -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = vec![
        SharedRoot::new_in_scope("Docs", "/home/u/Docs", DIRECT_EXPORT_SCOPE),
        SharedRoot::new_in_scope("Fotos", "/home/u/Fotos", DIRECT_EXPORT_SCOPE),
    ];
    profiles
}

fn peer() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("peer", "Peer", &iroh::SecretKey::from_bytes(&[5; 32]))
}

fn target_of(profiles: &ShareProfiles, device: &str) -> RightsTarget {
    let grant = profiles.grant_for(device).expect("grant");
    RightsTarget::Direct {
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
    }
}

fn rights_of(profiles: &ShareProfiles, device: &str) -> ConnectionRights {
    let grant = profiles.grant_for(device).expect("grant");
    profiles
        .current_direct_rights(&grant.device_id, &grant.public_key)
        .expect("rights")
}

fn session(profiles: &ShareProfiles, device: &str) -> SessionAuthorization {
    let grant = profiles.grant_for(device).expect("grant");
    SessionAuthorization::direct(profiles.default_direct_exports.clone(), grant)
}

/// A device accepted (legacy request path) and confirmed with „Alles“.
fn legacy_device_with_all(profiles: &mut ShareProfiles) {
    crate::share::legacy_direct_request_decision::set_exact_grant(profiles, &peer(), true, NOW)
        .unwrap();
    let target = target_of(profiles, "peer");
    profiles
        .confirm_connection_rights(&target, &RightsPreset::All.rights(), NOW)
        .unwrap();
    assert!(rights_of(profiles, "peer").grants_anything());
}

/// A second request of the fixture's requester (another request id).
fn second_request() -> SignedDirectRequest {
    let target_public = iroh::SecretKey::from_bytes(&[2; 32]).public().to_string();
    SignedDirectRequest::sign_with_nonce(
        DirectRequestId::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
        "lookup-a",
        DirectPeerIdentity::from_secret(
            "requester-a",
            "Requester",
            &iroh::SecretKey::from_bytes(&[1; 32]),
        ),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        300,
        400,
        "second-request-nonce",
        None,
        &[0x66; 32],
        &iroh::SecretKey::from_bytes(&[1; 32]),
    )
    .unwrap()
}

#[test]
fn abgleich_task_readmitted_ledger_device_waits_without_its_old_rights() {
    let cases: [(&str, fn()); 3] = [
        (
            "abgleich_task_readmitted_ledger_device_waits_without_its_old_rights",
            abgleich_task_readmitted_ledger_device_waits_without_its_old_rights_case,
        ),
        (
            "abgleich_task_readmitted_legacy_device_takes_the_code_rights",
            abgleich_task_readmitted_legacy_device_takes_the_code_rights,
        ),
        (
            "abgleich_task_readmitted_legacy_device_waits_without_its_old_rights",
            abgleich_task_readmitted_legacy_device_waits_without_its_old_rights,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

fn abgleich_task_readmitted_ledger_device_waits_without_its_old_rights_case() {
    let first = ledger::request(None);
    let mut profiles = profiles_with_exports();
    profiles
        .record_incoming_direct_request("lookup-a", first.clone(), 110)
        .unwrap();
    profiles
        .record_direct_decision(
            ledger::decision(&first, DirectDecisionKind::Accepted, 1, 130),
            130,
        )
        .unwrap();
    let target = target_of(&profiles, "requester-a");
    profiles
        .confirm_connection_rights(&target, &RightsPreset::All.rights(), 140)
        .unwrap();
    profiles
        .record_direct_decision(
            ledger::decision(&first, DirectDecisionKind::Revoked, 2, 250),
            250,
        )
        .unwrap();
    assert_eq!(
        profiles.grant_for("requester-a").unwrap().state,
        DirectGrantState::Ignored
    );
    // Before W6 the grant came back with write and every folder.
    let second = second_request();
    profiles
        .record_incoming_direct_request("lookup-a", second.clone(), 300)
        .unwrap();
    profiles
        .record_direct_decision(
            ledger::decision(&second, DirectDecisionKind::Accepted, 1, 310),
            310,
        )
        .unwrap();
    let grant = profiles.grant_for("requester-a").unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert!(!grant.write);
    assert!(!grant.exec.grants_any());
    assert!(session(&profiles, "requester-a").exports.roots.is_empty());
    assert_eq!(
        profiles
            .pending_rights_of(&target)
            .map(|pending| pending.origin),
        Some(RightsOrigin::Readmitted)
    );
}

fn abgleich_task_readmitted_legacy_device_takes_the_code_rights() {
    let mut profiles = profiles_with_exports();
    legacy_device_with_all(&mut profiles);
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        false,
        NOW + 1,
    )
    .unwrap();
    profiles
        .set_code_rights(Some(RightsPreset::ReadOnly.rights()))
        .unwrap();
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        true,
        NOW + 2,
    )
    .unwrap();
    assert_eq!(
        rights_of(&profiles, "peer"),
        RightsPreset::ReadOnly.rights()
    );
    assert!(profiles.pending_rights.is_empty());
}

fn abgleich_task_readmitted_legacy_device_waits_without_its_old_rights() {
    let mut profiles = profiles_with_exports();
    legacy_device_with_all(&mut profiles);
    // Revoked: the grant is ignored.
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        false,
        NOW + 1,
    )
    .unwrap();
    assert_eq!(
        profiles.grant_for("peer").unwrap().state,
        DirectGrantState::Ignored
    );
    // A new request of the same device is accepted.
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        true,
        NOW + 2,
    )
    .unwrap();
    let grant = profiles.grant_for("peer").unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert_eq!(rights_of(&profiles, "peer"), ConnectionRights::none());
    assert!(session(&profiles, "peer").exports.roots.is_empty());
    assert!(!session(&profiles, "peer").may_write);
    let pending = profiles
        .pending_rights_of(&target_of(&profiles, "peer"))
        .expect("waits for confirmation");
    assert_eq!(pending.origin, RightsOrigin::Readmitted);
}

fn auth_state(profiles: &ShareProfiles) -> crate::share::types::ShareAuthState {
    let secret = iroh::SecretKey::from_bytes(&[1; 32]);
    let local = DirectPeerIdentity::from_secret("local-device", "Local", &secret);
    let identity = ShareIdentity {
        device_id: local.device_id,
        device_name: local.device_name,
        direct_lookup_id: "local-lookup".into(),
        public_key: local.public_key,
        fingerprint: local.fingerprint,
        node_id: local.node_id,
        iroh_secret: secret,
        direct_secret: [7; 32],
    };
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
fn abgleich_task_narrower_folders_end_work_also_with_an_exec_change() {
    let mut profiles = profiles_with_exports();
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        true,
        NOW,
    )
    .unwrap();
    let target = target_of(&profiles, "peer");
    profiles
        .confirm_connection_rights(&target, &RightsPreset::FilesExec.rights(), NOW)
        .unwrap();
    let only_docs = ConnectionRights {
        folders: FolderChoice::Selected {
            paths: vec!["/home/u/Docs".into()],
            connections: false,
        },
        ..RightsPreset::ReadWrite.rights()
    };
    // The review's case: write stays, commands and terminal narrow, Fotos
    // goes, all in one confirmation.
    let mut narrowed = profiles.clone();
    narrowed
        .confirm_connection_rights(&target, &only_docs, NOW + 1)
        .unwrap();
    let grant = profiles.grant_for("peer").unwrap();
    let restrictions = authorization_restrictions(&auth_state(&profiles), &auth_state(&narrowed));
    assert!(
        restrictions.affects("direct", "", &grant.public_key, &grant.node_id),
        "running sessions, leases and transfers of the device end"
    );
    assert!(restrictions.affects_exec("direct", "", &grant.public_key, &grant.node_id));
    // Folders alone, and folders with a write revocation, also end them.
    for rights in [
        ConnectionRights {
            folders: only_docs.folders.clone(),
            ..RightsPreset::FilesExec.rights()
        },
        ConnectionRights {
            folders: only_docs.folders.clone(),
            ..RightsPreset::ReadOnly.rights()
        },
    ] {
        let mut candidate = profiles.clone();
        candidate
            .confirm_connection_rights(&target, &rights, NOW + 1)
            .unwrap();
        let restrictions =
            authorization_restrictions(&auth_state(&profiles), &auth_state(&candidate));
        assert!(restrictions.affects("direct", "", &grant.public_key, &grant.node_id));
    }
    // A widening restricts nothing.
    let mut wider = profiles.clone();
    wider
        .confirm_connection_rights(&target, &RightsPreset::All.rights(), NOW + 1)
        .unwrap();
    let restrictions = authorization_restrictions(&auth_state(&profiles), &auth_state(&wider));
    assert!(!restrictions.affects("direct", "", &grant.public_key, &grant.node_id));
}

#[test]
fn abgleich_task_confirm_names_exactly_the_exec_grants_it_changes() {
    let mut profiles = profiles_with_exports();
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        true,
        NOW,
    )
    .unwrap();
    let target = target_of(&profiles, "peer");
    let mut granted = profiles.clone();
    granted
        .confirm_connection_rights(&target, &RightsPreset::All.rights(), NOW)
        .unwrap();
    let changed = changed_exec_grants(&profiles, &granted).unwrap();
    assert_eq!(changed.len(), 1);
    let (crate::share::ExecGrantTarget::Direct { device_id, .. }, policy) = &changed[0] else {
        panic!("a Direct grant")
    };
    assert_eq!(device_id, "peer");
    assert_eq!(
        policy.decision_at(Capability::ScreenControl, NOW),
        CapabilityDecision::Allow
    );
    // Folders and write only: no Exec grant changes.
    let mut files = granted.clone();
    files
        .confirm_connection_rights(
            &target,
            &ConnectionRights {
                write: false,
                ..RightsPreset::All.rights()
            },
            NOW + 1,
        )
        .unwrap();
    assert!(changed_exec_grants(&granted, &files).unwrap().is_empty());
}

/// Storage of one profile file in memory.
#[derive(Default)]
struct Memory {
    profiles: Option<String>,
    saves: usize,
}

impl ProfilePersistence for Memory {
    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        Ok(self.profiles.clone())
    }

    fn save_profiles(
        &mut self,
        contents: &str,
        _expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String> {
        self.saves += 1;
        self.profiles = Some(contents.into());
        Ok(ProfileRevision::from_contents(contents))
    }

    fn save_secret(&mut self, _account: &str, _secret: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_secret(&mut self, _account: &str) -> Result<(), String> {
        Ok(())
    }
}

fn written_version(memory: &Memory) -> u64 {
    let value: serde_json::Value =
        serde_json::from_str(memory.profiles.as_deref().unwrap()).unwrap();
    value["schema_version"].as_u64().unwrap()
}

/// What versions up to 0.5.189 accept (`OLDEST` 3 to `SHARE` 8).
fn older_version_reads(version: u64) -> bool {
    (3..=8).contains(&version)
}

#[test]
fn abgleich_task_rights_records_are_written_so_older_versions_refuse_them() {
    // Without rights records: version 8, readable by older versions.
    let mut memory = Memory::default();
    let mut plain = profiles_with_exports();
    plain.save_with(&mut memory).unwrap();
    assert_eq!(written_version(&memory), 8);
    assert!(older_version_reads(written_version(&memory)));

    // A new connection writes a folder selection (none) and „Rechte offen“.
    let mut profiles = profiles_with_exports();
    crate::share::legacy_direct_request_decision::set_exact_grant(
        &mut profiles,
        &peer(),
        true,
        NOW,
    )
    .unwrap();
    let mut memory = Memory::default();
    profiles.save_with(&mut memory).unwrap();
    assert_eq!(written_version(&memory), 9);
    assert!(
        !older_version_reads(written_version(&memory)),
        "an older version must not read the device as one with every folder"
    );
    // Each record alone raises the version.
    let mut code = profiles_with_exports();
    code.set_code_rights(Some(RightsPreset::ReadOnly.rights()))
        .unwrap();
    assert_eq!(code.stored_schema_version(), 9);
    let mut room = profiles_with_exports();
    room.rooms.push(crate::share::RoomProfile {
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
    assert_eq!(room.stored_schema_version(), 8);
    room.apply_room_connection_rights("room-1", &RightsPreset::FilesExec.rights(), NOW)
        .unwrap();
    assert_eq!(room.stored_schema_version(), 9, "member access");

    // This version reads 9 unchanged and does not rewrite it on load.
    let saves = memory.saves;
    let loaded = ShareProfiles::load_checked_with(None, &mut memory).unwrap();
    assert_eq!(memory.saves, saves);
    assert_eq!(
        loaded.default_direct_exports.device_folders,
        profiles.default_direct_exports.device_folders
    );
    assert_eq!(loaded.pending_rights, profiles.pending_rights);
    // And 8 (and every older version) as before.
    let mut older = Memory::default();
    plain.save_with(&mut older).unwrap();
    let saves = older.saves;
    let loaded = ShareProfiles::load_checked_with(None, &mut older).unwrap();
    assert_eq!(older.saves, saves);
    assert_eq!(loaded.schema_version, 8);
    assert_eq!(
        loaded.default_direct_exports.roots,
        plain.default_direct_exports.roots
    );
}

#[test]
fn abgleich_task_test_store_joins_rooms_like_the_production_store() {
    use crate::share::discovery_relation_store::{InMemoryRelationStore, RelationStore};
    let mut store = InMemoryRelationStore::new(profiles_with_exports());
    let material =
        crate::share::room_relation::RoomRelationMaterial::new("joined-room", vec![9; 32]).unwrap();
    store.persist_room(&material, "Team").unwrap();
    let room = &store.profiles().rooms[0];
    assert!(room.exports.roots.is_empty(), "no Direct folder copied");
    assert!(!room.exports.shares_connections());
    let pending = store
        .profiles()
        .pending_rights_of(&RightsTarget::Room {
            profile_id: room.id.clone(),
        })
        .expect("waits for its rights");
    assert_eq!(pending.origin, RightsOrigin::RoomJoined);
}
