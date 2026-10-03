use super::core::{b64, random_token};
use super::profiles::{
    direct_contact_secret_account, room_secret_account, DirectCode, ProfileRevision, RoomCode,
    ShareProfiles, LEGACY_SHARE_PROFILE_VERSION, OLDEST_SHARE_PROFILE_VERSION,
    PREVIOUS_SHARE_PROFILE_VERSION, REMOVED_PEERS_PREVIOUS_VERSION, SHARE_PROFILE_VERSION,
    TOMBSTONE_SHARE_PROFILE_VERSION,
};
use super::types::{
    DirectAccessState, DirectContact, DirectGrantState, PeerPresence, RoomProfile, ShareStatus,
};

pub(super) const MAX_PROFILE_BYTES: u64 = 1024 * 1024;

pub(super) fn encode_profiles(profiles: &ShareProfiles) -> Result<String, String> {
    struct Buffer(Vec<u8>);
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if (self.0.len() as u64).saturating_add(bytes.len() as u64) > MAX_PROFILE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Share profiles exceed their byte budget; history was not discarded",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer_pretty(&mut buffer, profiles)
        .map_err(|error| format!("Share-Profile kodieren: {error}"))?;
    String::from_utf8(buffer.0).map_err(|error| format!("Share-Profile kodieren: {error}"))
}

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
        let (mut profiles, revision, raw) = match loaded {
            Some(raw) => {
                if raw.len() as u64 > MAX_PROFILE_BYTES {
                    return Err("Share profiles exceed their byte budget".into());
                }
                let revision = ProfileRevision::from_contents(&raw);
                let profiles = serde_json::from_str::<ShareProfiles>(&raw)
                    .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
                let value = serde_json::from_str::<serde_json::Value>(&raw)
                    .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
                (profiles, revision, Some(value))
            }
            None => (ShareProfiles::default(), ProfileRevision::Missing, None),
        };
        profiles.storage_revision = revision;
        let old_version = profiles.schema_version;
        match profiles.schema_version {
            SHARE_PROFILE_VERSION => {}
            REMOVED_PEERS_PREVIOUS_VERSION
            | TOMBSTONE_SHARE_PROFILE_VERSION
            | PREVIOUS_SHARE_PROFILE_VERSION => {
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
        if let Some(raw) = raw {
            let legacy_connections = profiles.default_direct_exports.include_connections
                || profiles
                    .rooms
                    .iter()
                    .any(|room| room.exports.include_connections);
            let accounts = if legacy_connections {
                storage.saved_connection_accounts()?.ok_or_else(||
                    "Gespeicherte Verbindungskonten fuer die Freigabenmigration nicht verfuegbar".to_string())?
            } else {
                Vec::new()
            };
            let migrated =
                profiles.migrate_export_policy(&raw, default_home.as_deref(), &accounts)?;
            if migrated || old_version != profiles.schema_version {
                // The old configuration remains intact if the write fails;
                // no uncommitted migration is handed to the host runtime.
                profiles.save_with(storage)?;
            }
        }
        Ok(profiles)
    }

    pub(super) fn save_with(
        &mut self,
        storage: &mut impl ProfilePersistence,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.reconcile_legacy_grants(super::core::now_secs());
        candidate.recompute_all_identity_conflicts();
        candidate
            .validate_direct_ledger()
            .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
        candidate
            .validate_legacy_direct_requests()
            .map_err(|error| format!("Share-Profile sind beschaedigt: {error}"))?;
        let contents = encode_profiles(&candidate)?;
        let expected = match &self.storage_revision {
            ProfileRevision::Untracked => ProfileRevision::Missing,
            revision => revision.clone(),
        };
        let revision = storage
            .save_profiles(&contents, &expected)
            .map_err(|error| format!("Share-Profile speichern: {error}"))?;
        candidate.storage_revision = revision;
        *self = candidate;
        Ok(())
    }

    pub(super) fn persist_replacement_with(
        &mut self,
        mut candidate: ShareProfiles,
        storage: &mut impl ProfilePersistence,
    ) -> Result<(), String> {
        candidate.schema_version = SHARE_PROFILE_VERSION;
        candidate.storage_revision = self.storage_revision.clone();
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
            lan_candidates: Vec::new(),
            lan_seen_at: None,
            lan_uplink: None,
            relation: Default::default(),
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
        let mut candidate = self.clone();
        if candidate
            .forget_direct_peer(contact_id, super::core::now_secs())
            .is_none()
        {
            return Ok(ProfileChange::default());
        }
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
            exports: super::export_config::ShareExportConfig::default(),
            policy: super::room_relation::RoomPolicy::new_room(),
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
    /// `None` cannot silently turn the legacy all-connections flag into
    /// an empty explicit set. The system adapter supplies the current list.
    fn saved_connection_accounts(&mut self) -> Result<Option<Vec<String>>, String> {
        Ok(None)
    }
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
#[path = "profile_persistence_tests.rs"]
mod tests;
