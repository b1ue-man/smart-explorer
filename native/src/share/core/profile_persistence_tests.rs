use std::collections::HashMap;

use super::{ProfilePersistence, ProfileRevision, ShareProfiles, SHARE_PROFILE_VERSION};
use crate::share::{
    DirectGrant, DirectGrantState, ExecGrant, RoomMember, RoomProfile, ShareExportConfig,
    ShareStatus,
};

#[derive(Default)]
struct FakePersistence {
    profiles: Option<String>,
    secrets: HashMap<String, String>,
    fail_profile_save: bool,
    fail_secret_save: bool,
    saved_accounts: Vec<String>,
}

impl ProfilePersistence for FakePersistence {
    fn saved_connection_accounts(&mut self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(self.saved_accounts.clone()))
    }

    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        Ok(self.profiles.clone())
    }

    fn save_profiles(
        &mut self,
        contents: &str,
        expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String> {
        if self.fail_profile_save {
            Err("disk full".into())
        } else {
            let current = self
                .profiles
                .as_deref()
                .map(ProfileRevision::from_contents)
                .unwrap_or(ProfileRevision::Missing);
            if !matches!(expected, ProfileRevision::Untracked) && expected != &current {
                return Err("Share profiles changed concurrently".into());
            }
            self.profiles = Some(contents.to_string());
            Ok(ProfileRevision::from_contents(contents))
        }
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        if self.fail_secret_save {
            Err("secure store unavailable".into())
        } else {
            self.secrets.insert(account.to_string(), secret.to_string());
            Ok(())
        }
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        self.secrets.remove(account);
        Ok(())
    }
}

#[test]
fn failed_direct_profile_write_rolls_back_contact_and_secret() {
    let mut profiles = ShareProfiles::default();
    let mut storage = FakePersistence {
        fail_profile_save: true,
        ..FakePersistence::default()
    };
    let code = format!("SE-D3-lookup-{}-{}-node", "11".repeat(32), "22".repeat(16));
    assert!(profiles
        .add_direct_from_code_with(&code, "Peer", &mut storage)
        .is_err());
    assert!(profiles.direct_contacts.is_empty());
    assert!(storage.secrets.is_empty());
}

#[test]
fn failed_secret_write_never_adds_a_direct_contact() {
    let mut profiles = ShareProfiles::default();
    let mut storage = FakePersistence {
        fail_secret_save: true,
        ..FakePersistence::default()
    };
    let code = format!("SE-D3-lookup-{}-{}-node", "11".repeat(32), "22".repeat(16));
    assert!(profiles
        .add_direct_from_code_with(&code, "Peer", &mut storage)
        .is_err());
    assert!(profiles.direct_contacts.is_empty());
    assert!(storage.profiles.is_none());
}

#[test]
fn persisted_empty_export_list_is_not_replaced_with_home() {
    let mut storage = FakePersistence::default();
    let mut profiles =
        ShareProfiles::load_checked_with(Some("/home/alice".into()), &mut storage)
            .expect("load first-run profiles");
    assert!(profiles.default_direct_exports.roots.is_empty());
    profiles.default_direct_exports.roots.clear();
    profiles
        .save_with(&mut storage)
        .expect("persist empty list");

    let reloaded = ShareProfiles::load_checked_with(Some("/home/alice".into()), &mut storage)
        .expect("reload explicit empty list");
    assert!(reloaded.default_direct_exports.roots.is_empty());
}

#[test]
fn stale_profile_revision_cannot_overwrite_a_newer_save() {
    let mut storage = FakePersistence::default();
    let mut first = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    let mut stale = first.clone();
    first.auto_connect = false;
    first.save_with(&mut storage).unwrap();
    stale.auto_connect = true;
    let error = stale.save_with(&mut storage).unwrap_err();
    assert!(error.contains("concurrently"));
    let current = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert!(!current.auto_connect);
}

#[test]
fn review_task_fc1_implicit_home_and_connections_migrate_once_without_new_grants() {
    let raw = serde_json::json!({
        "schema_version": SHARE_PROFILE_VERSION,
        "default_direct_exports": {"roots": [
            {"label": "Home", "path": "/home/alice"},
            {"label": "Work", "path": "//nas/share/literal"},
            {"label": "Home", "path": "/home/alice", "access": "read_write"},
            {"label": "Home", "path": "/home/alice\\"}
        ], "include_connections": true, "shared_connections": [
            {"account": "ftp://u@host:21/", "access": "read_only"}
        ]},
        "rooms": [{"id": "p1", "name": "Old room", "room_id": "r1", "auto_join": false,
            "last_seen": null, "exports": {"roots": [{"label":"Archive", "path":"/archive"}]}}]
    });
    let mut storage = FakePersistence {
        profiles: Some(raw.to_string()),
        saved_accounts: vec!["ftp://u@host:21/".into(), "sftp://u@nas:22/literal".into()],
        ..FakePersistence::default()
    };
    let migrated = ShareProfiles::load_checked_with(Some("/home/alice".into()), &mut storage).unwrap();
    let config = &migrated.default_direct_exports;
    assert_eq!(config.roots[0].access, crate::share::ExportAccess::ReadOnly);
    assert_eq!(config.roots[1].access, crate::share::ExportAccess::ReadWrite);
    assert_eq!(config.roots[1].path, "//nas/share/literal");
    assert_eq!(config.roots[2].access, crate::share::ExportAccess::ReadWrite);
    assert_eq!(config.roots[3].access, crate::share::ExportAccess::ReadWrite);
    assert_eq!(config.roots[3].path, "/home/alice\\");
    assert!(!config.include_connections);
    assert_eq!(config.connection_access("ftp://u@host:21/"), Some(crate::share::ExportAccess::ReadOnly));
    assert_eq!(config.connection_access("sftp://u@nas:22/literal"), Some(crate::share::ExportAccess::ReadWrite));
    assert!(migrated.rooms[0].policy.members_may_write);
    assert_eq!(migrated.rooms[0].exports.roots[0].access, crate::share::ExportAccess::ReadWrite);
    assert!(migrated.direct_grants.is_empty());
    assert_eq!(migrated.auto_home_migrations.len(), 1);
    let committed = storage.profiles.clone();
    storage.saved_accounts.push("webdav://u@later:443/".into());
    let reloaded = ShareProfiles::load_checked_with(Some("/home/alice".into()), &mut storage).unwrap();
    assert_eq!(reloaded.auto_home_migrations.len(), 1);
    assert_eq!(reloaded.default_direct_exports.connection_access("webdav://u@later:443/"), None);
    assert_eq!(storage.profiles, committed);
}

#[test]
fn review_task_fc1_failed_migration_is_retryable_and_returns_no_runtime_profile() {
    let raw = serde_json::json!({"schema_version": SHARE_PROFILE_VERSION,
        "default_direct_exports": {"roots": [{"label":"Home", "path":"/home/alice"}]}}).to_string();
    let mut storage = FakePersistence { profiles: Some(raw.clone()), fail_profile_save: true,
        ..FakePersistence::default() };
    assert!(ShareProfiles::load_checked_with(None, &mut storage).unwrap_err().contains("Home-Ort"));
    assert_eq!(storage.profiles.as_deref(), Some(raw.as_str()));
    assert!(ShareProfiles::load_checked_with(Some("/home/alice".into()), &mut storage).is_err());
    assert_eq!(storage.profiles.as_deref(), Some(raw.as_str()));
    storage.fail_profile_save = false;
    let migrated = ShareProfiles::load_checked_with(Some("/home/alice".into()), &mut storage).unwrap();
    assert_eq!(migrated.default_direct_exports.roots[0].access, crate::share::ExportAccess::ReadOnly);
    assert_ne!(storage.profiles.as_deref(), Some(raw.as_str()));
}

#[test]
fn review_task_fc1_new_room_has_no_inherited_exports() {
    let mut storage = FakePersistence::default();
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots.push(crate::share::SharedRoot::new("Work", "/work"));
    let code = format!("SE-R3-room-{}", "11".repeat(32));
    let id = profiles.add_room_from_code_with(&code, "New", &mut storage).unwrap();
    let room = profiles.rooms.iter().find(|room| room.id == id).unwrap();
    assert!(room.exports.roots.is_empty());
    assert!(!room.exports.shares_connections());
    assert!(!room.policy.members_may_write);
}

#[test]
fn review_task_fc1_byte_budget_and_runtime_flags_preserve_more_than_64_denials() {
    let mut storage = FakePersistence::default();
    let mut profiles = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    for index in 0..70 {
        profiles.record_removed_direct_peer(&crate::share::DirectPeerIdentity {
            device_id: format!("d{index}"), device_name: String::new(), public_key: format!("key{index}"),
            node_id: format!("node{index}"), fingerprint: format!("fp{index}"),
        }, index);
    }
    profiles.save_with(&mut storage).unwrap();
    let old = storage.profiles.clone();
    let mut untracked = ShareProfiles::default();
    assert!(untracked.save_with(&mut storage).is_err());
    assert_eq!(storage.profiles, old);
    profiles.auto_home_migrations.push(crate::share::AutoHomeMigration {
        scope: "direct".into(), path: "\\".repeat(super::MAX_PROFILE_BYTES as usize / 2 + 1),
    });
    assert!(profiles.save_with(&mut storage).is_err());
    assert_eq!(storage.profiles, old);
    let mut reloaded = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert_eq!(reloaded.removed_direct_peers.len(), 70);
    reloaded.auto_connect = false;
    reloaded.save_with(&mut storage).unwrap();
    let persisted = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert_eq!(persisted.removed_direct_peers.len(), 70);
    assert!(persisted.removed_direct_peer_for_device("d0").is_some());
    storage.profiles = Some(" ".repeat(super::MAX_PROFILE_BYTES as usize + 1));
    assert!(ShareProfiles::load_checked_with(None, &mut storage).unwrap_err().contains("byte budget"));
}

#[test]
fn v3_and_v4_profiles_migrate_to_current_with_exec_default_denied() {
    let mut legacy = ShareProfiles {
        auto_connect: false,
        ..ShareProfiles::default()
    };
    let enabled = ExecGrant {
        enabled: true,
        policy_revision: 9,
        changed_at: 7,
        ..ExecGrant::default()
    };
    legacy.direct_grants.push(direct_grant(enabled.clone()));
    legacy.rooms.push(room_with_member(enabled));

    for version in [3, 4] {
        let mut value = serde_json::to_value(&legacy).unwrap();
        value["schema_version"] = serde_json::json!(version);
        value["default_direct_exports"]["allow_exec"] = serde_json::json!(true);
        value["rooms"][0]["exports"]["allow_exec"] = serde_json::json!(true);
        if version == 3 {
            value.as_object_mut().unwrap().remove("direct_requests");
        }
        let mut storage = FakePersistence {
            profiles: Some(serde_json::to_string_pretty(&value).unwrap()),
            ..FakePersistence::default()
        };

        let mut migrated = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
        assert_eq!(migrated.schema_version, SHARE_PROFILE_VERSION);
        assert!(!migrated.auto_connect);
        assert!(!migrated.direct_grants[0].exec.enabled);
        assert!(!migrated.rooms[0].members[0].exec.enabled);

        migrated.save_with(&mut storage).unwrap();
        let persisted = storage.profiles.as_deref().unwrap();
        assert!(!persisted.contains("allow_exec"));
    }
}

#[test]
fn profile_versions_older_than_v3_and_newer_than_v6_fail_closed() {
    for version in [2, SHARE_PROFILE_VERSION + 1] {
        let mut value = serde_json::to_value(ShareProfiles::default()).unwrap();
        value["schema_version"] = serde_json::json!(version);
        let mut storage = FakePersistence {
            profiles: Some(serde_json::to_string(&value).unwrap()),
            ..FakePersistence::default()
        };
        let error = ShareProfiles::load_checked_with(None, &mut storage).unwrap_err();
        assert!(error.contains("Nicht unterstuetzte Share-Profilversion"));
    }
}

fn direct_grant(exec: ExecGrant) -> DirectGrant {
    DirectGrant {
        device_id: "device-a".into(),
        device_name: "Device A".into(),
        public_key: "key-a".into(),
        fingerprint: "fingerprint-a".into(),
        node_id: "node-a".into(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec,
        write: false,
    }
}

fn room_with_member(exec: ExecGrant) -> RoomProfile {
    RoomProfile {
        id: "profile-a".into(),
        name: "Room A".into(),
        room_id: "room-a".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![RoomMember {
            device_id: "device-b".into(),
            device_name: "Device B".into(),
            fingerprint: "fingerprint-b".into(),
            public_key: "key-b".into(),
            node_id: "node-b".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: ShareStatus::Waiting,
            blocked: false,
            exec,
            presence: None,
            relation: Default::default(),
        }],
        exports: ShareExportConfig::default(),
        policy: crate::share::RoomPolicy::new_room(),
    }
}
