use super::core::{b64, random_token};
use super::fs::SharedRoot;
use super::profiles::{
    direct_contact_secret_account, room_secret_account, DirectCode, ProfileRevision, RoomCode,
    ShareProfiles, LEGACY_SHARE_PROFILE_VERSION, OLDEST_SHARE_PROFILE_VERSION,
    PREVIOUS_SHARE_PROFILE_VERSION, SHARE_PROFILE_VERSION, TOMBSTONE_SHARE_PROFILE_VERSION,
};
use super::types::{
    DirectAccessState, DirectContact, DirectGrantState, PeerPresence, RoomProfile, ShareStatus,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileChange {
    pub changed: bool,
    pub cleanup_warning: Option<String>,
}

impl ShareProfiles {
    pub(super) fn load_checked_with(
        default_home: Option<String>,
        storage: &mut impl ProfilePersistence,
    ) -> Result<Self, String> {
        let loaded = storage
            .load_profiles()
            .map_err(|error| format!("Share-Profile lesen: {error}"))?;
        let missing = loaded.is_none();
        let (mut profiles, revision) = match loaded {
            Some(raw) => {
                let revision = ProfileRevision::from_contents(&raw);
                let profiles = serde_json::from_str::<ShareProfiles>(&raw)
                    .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
                (profiles, revision)
            }
            None => (ShareProfiles::default(), ProfileRevision::Missing),
        };
        profiles.storage_revision = revision;
        match profiles.schema_version {
            SHARE_PROFILE_VERSION => {}
            TOMBSTONE_SHARE_PROFILE_VERSION | PREVIOUS_SHARE_PROFILE_VERSION => {
                profiles.schema_version = SHARE_PROFILE_VERSION;
            }
            OLDEST_SHARE_PROFILE_VERSION | LEGACY_SHARE_PROFILE_VERSION => {
                profiles.schema_version = SHARE_PROFILE_VERSION;
                profiles.reset_exec_for_legacy_migration();
            }
            version => {
                return Err(format!(
                    "Nicht unterstuetzte Share-Profilversion {version} (erwartet {OLDEST_SHARE_PROFILE_VERSION} bis {SHARE_PROFILE_VERSION})"
                ));
            }
        }
        profiles.reconcile_legacy_grants(super::core::now_secs());
        profiles.recompute_all_identity_conflicts();
        profiles
            .validate_direct_ledger()
            .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
        profiles
            .validate_legacy_direct_requests()
            .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
        // Existing empty exports are an explicit deny-all configuration.
        if missing && profiles.default_direct_exports.roots.is_empty() {
            if let Some(home) = default_home {
                profiles.default_direct_exports.roots.push(SharedRoot {
                    label: "Home".to_string(),
                    path: home,
                });
            }
        }
        Ok(profiles)
    }

    pub(super) fn save_with(
        &mut self,
        storage: &mut impl ProfilePersistence,
    ) -> Result<(), String> {
        self.reconcile_legacy_grants(super::core::now_secs());
        self.recompute_all_identity_conflicts();
        self.validate_legacy_direct_requests()
            .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
        let contents = serde_json::to_string_pretty(self)
            .map_err(|error| format!("Share-Profile kodieren: {error}"))?;
        let revision = storage
            .save_profiles(&contents, &self.storage_revision)
            .map_err(|error| format!("Share-Profile speichern: {error}"))?;
        self.storage_revision = revision;
        Ok(())
    }

    pub(super) fn persist_replacement_with(
        &mut self,
        mut candidate: ShareProfiles,
        storage: &mut impl ProfilePersistence,
    ) -> Result<(), String> {
        candidate.schema_version = SHARE_PROFILE_VERSION;
        candidate.save_with(storage)?;
        *self = candidate;
        Ok(())
    }

    pub(super) fn add_direct_from_code_with(
        &mut self,
        code: &str,
        name: &str,
        storage: &mut impl ProfilePersistence,
    ) -> Result<String, String> {
        let mut parsed = DirectCode::parse(code)?;
        if self
            .direct_contacts
            .iter()
            .any(|contact| contact.lookup_id == parsed.lookup_id)
        {
            return Err("Direktgeraet ist bereits gespeichert".into());
        }
        let id = random_token(10)
            .map_err(|error| format!("Sichere Direktkontakt-ID erzeugen: {error}"))?;
        let account = direct_contact_secret_account(&id);
        storage
            .save_secret(&account, &b64(&parsed.secret))
            .map_err(|error| format!("Direkt-Secret speichern: {error}"))?;
        let label = if name.trim().is_empty() {
            format!(
                "Direkt {}",
                &parsed.fingerprint[..parsed.fingerprint.len().min(8)]
            )
        } else {
            name.trim().to_string()
        };
        let mut candidate = self.clone();
        candidate.direct_contacts.push(DirectContact {
            id: id.clone(),
            display_name: label,
            lookup_id: std::mem::take(&mut parsed.lookup_id),
            expected_fingerprint: std::mem::take(&mut parsed.fingerprint),
            expected_node_id: std::mem::take(&mut parsed.node_id),
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::WaitingForAccess,
            last_error: None,
            presence: None,
            access_state: DirectAccessState::Pending,
            request_sent_at: Some(super::core::now_secs()),
            accepted_at: None,
            accepted_public_key: None,
        });
        if let Err(error) = candidate.save_with(storage) {
            return Err(cleanup_new_secret(error, storage, &account));
        }
        *self = candidate;
        Ok(id)
    }

    pub(super) fn set_direct_grant_persisted_with(
        &mut self,
        presence: &PeerPresence,
        state: DirectGrantState,
        storage: &mut impl ProfilePersistence,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.set_direct_grant(presence, state)?;
        candidate.save_with(storage)?;
        *self = candidate;
        Ok(())
    }

    pub(super) fn remove_direct_contact_with(
        &mut self,
        contact_id: &str,
        storage: &mut impl ProfilePersistence,
    ) -> Result<ProfileChange, String> {
        if !self
            .direct_contacts
            .iter()
            .any(|contact| contact.id == contact_id)
        {
            return Ok(ProfileChange::default());
        }
        let mut candidate = self.clone();
        candidate
            .direct_contacts
            .retain(|contact| contact.id != contact_id);
        candidate
            .direct_requests
            .retain(|request| request.contact_id.as_deref() != Some(contact_id));
        candidate.save_with(storage)?;
        *self = candidate;
        let cleanup_warning = storage
            .delete_secret(&direct_contact_secret_account(contact_id))
            .err()
            .map(|error| format!("Kontakt entfernt, aber sein Secret blieb gespeichert: {error}"));
        Ok(ProfileChange {
            changed: true,
            cleanup_warning,
        })
    }

    pub(super) fn add_room_from_code_with(
        &mut self,
        code: &str,
        name: &str,
        storage: &mut impl ProfilePersistence,
    ) -> Result<String, String> {
        let material = RoomCode::parse(code)?.into_relation_material()?;
        if let Some(existing) = self
            .rooms
            .iter()
            .find(|room| room.room_id == material.room_id())
        {
            return Ok(existing.id.clone());
        }
        let id =
            random_token(10).map_err(|error| format!("Sichere Raumprofil-ID erzeugen: {error}"))?;
        let account = room_secret_account(&id);
        storage
            .save_secret(&account, &b64(material.secret()))
            .map_err(|error| format!("Raum-Secret speichern: {error}"))?;
        let mut candidate = self.clone();
        candidate.rooms.push(RoomProfile {
            id: id.clone(),
            name: if name.trim().is_empty() {
                "Raum".to_string()
            } else {
                name.trim().to_string()
            },
            room_id: material.room_id().to_string(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: Vec::new(),
            exports: self.default_direct_exports.clone(),
        });
        if let Err(error) = candidate.save_with(storage) {
            return Err(cleanup_new_secret(error, storage, &account));
        }
        *self = candidate;
        Ok(id)
    }

    pub(super) fn remove_room_with(
        &mut self,
        room_id: &str,
        storage: &mut impl ProfilePersistence,
    ) -> Result<ProfileChange, String> {
        if !self.rooms.iter().any(|room| room.id == room_id) {
            return Ok(ProfileChange::default());
        }
        let mut candidate = self.clone();
        candidate.rooms.retain(|room| room.id != room_id);
        candidate.save_with(storage)?;
        *self = candidate;
        let cleanup_warning = storage
            .delete_secret(&room_secret_account(room_id))
            .err()
            .map(|error| format!("Raum entfernt, aber sein Secret blieb gespeichert: {error}"));
        Ok(ProfileChange {
            changed: true,
            cleanup_warning,
        })
    }
}

fn cleanup_new_secret(
    error: String,
    storage: &mut impl ProfilePersistence,
    account: &str,
) -> String {
    match storage.delete_secret(account) {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; neues Secret konnte nicht entfernt werden: {cleanup}"),
    }
}

pub(super) trait ProfilePersistence {
    fn load_profiles(&mut self) -> Result<Option<String>, String>;
    fn save_profiles(
        &mut self,
        contents: &str,
        expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String>;
    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String>;
    fn delete_secret(&mut self, account: &str) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
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
    }

    impl ProfilePersistence for FakePersistence {
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
        assert_eq!(profiles.default_direct_exports.roots.len(), 1);
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
            }],
            exports: ShareExportConfig::default(),
        }
    }
}
