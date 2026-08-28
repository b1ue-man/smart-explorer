use super::core::random_token;
use super::profile_persistence::ProfileChange;
use super::profile_store::{
    credential_matches, delete_credential_verified, prepare_unique_credential, SecretString,
};
use super::profiles::{
    direct_contact_secret_account, room_secret_account, DirectCode, RoomCode, ShareProfiles,
};
use super::room_relation::{
    canonical_room_display_name, RoomPersistenceOutcome, RoomRelationMaterial,
};
use super::types::{DirectAccessState, DirectContact, RoomProfile, ShareStatus};

impl ShareProfiles {
    /// Add a direct contact against the newest profile revision. The relation
    /// secret is prepared once under a unique account; CAS retries only replay
    /// the deterministic profile mutation.
    pub fn add_direct_from_code_persisted(
        default_home: Option<String>,
        code: &str,
        name: &str,
    ) -> Result<(Self, String), String> {
        let parsed = DirectCode::parse(code)?;
        let generated_id = random_token(10)
            .map_err(|error| format!("Sichere Direktkontakt-ID erzeugen: {error}"))?;
        let account = direct_contact_secret_account(&generated_id);
        let secret = SecretString::encoded(&parsed.secret);
        prepare_unique_credential(&account, &secret)?;
        let label = if name.trim().is_empty() {
            format!(
                "Direkt {}",
                &parsed.fingerprint[..parsed.fingerprint.len().min(8)]
            )
        } else {
            name.trim().to_string()
        };
        let now = super::core::now_secs();
        let mut selected_id = generated_id.clone();
        let result = Self::mutate_persisted(default_home, |profiles| {
            if let Some(existing) = profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.lookup_id == parsed.lookup_id)
            {
                selected_id = existing.id.clone();
                return Ok(());
            }
            selected_id = generated_id.clone();
            profiles.direct_contacts.push(DirectContact {
                id: generated_id.clone(),
                display_name: label.clone(),
                lookup_id: parsed.lookup_id.clone(),
                expected_fingerprint: parsed.fingerprint.clone(),
                expected_node_id: parsed.node_id.clone(),
                remote_device_id: None,
                remote_public_key: None,
                auto_connect: true,
                auto_open: false,
                last_seen: None,
                status: ShareStatus::WaitingForAccess,
                last_error: None,
                presence: None,
                access_state: DirectAccessState::Pending,
                request_sent_at: Some(now),
                accepted_at: None,
                accepted_public_key: None,
            });
            Ok(())
        });
        let profiles = match result {
            Ok(profiles) => profiles,
            Err(error) => return Err(cleanup_prepared_secret(error, &account)),
        };
        if selected_id != generated_id {
            finish_rebased_credential(
                &account,
                &direct_contact_secret_account(&selected_id),
                &secret,
                "Direktgeraet ist bereits mit anderem Secret gespeichert",
            )?;
        }
        Ok((profiles, selected_id))
    }

    pub fn add_room_from_code_persisted(
        default_home: Option<String>,
        code: &str,
        name: &str,
    ) -> Result<(Self, String), String> {
        let material = RoomCode::parse(code)?.into_relation_material()?;
        let (profiles, outcome) =
            Self::add_room_material_persisted(default_home, &material, name)?;
        Ok((profiles, outcome.room_profile_id().to_string()))
    }

    /// Persist authenticated Room material without constructing a secret-bearing
    /// RoomCode string. The credential is installed before the profile CAS and
    /// verified against an existing canonical profile after any rebase.
    pub fn add_room_material_persisted(
        default_home: Option<String>,
        material: &RoomRelationMaterial,
        name: &str,
    ) -> Result<(Self, RoomPersistenceOutcome), String> {
        let label = canonical_room_display_name(name).map_err(|error| error.to_string())?;
        let generated_id =
            random_token(10).map_err(|error| format!("Sichere Raumprofil-ID erzeugen: {error}"))?;
        let account = room_secret_account(&generated_id);
        let secret = SecretString::encoded(material.secret());
        prepare_unique_credential(&account, &secret)?;
        let mut selected_id = generated_id.clone();
        let mut changed = false;
        let result = Self::mutate_persisted(default_home, |profiles| {
            if let Some(existing) = profiles
                .rooms
                .iter()
                .find(|room| room.room_id == material.room_id())
            {
                selected_id = existing.id.clone();
                changed = false;
                return Ok(());
            }
            selected_id = generated_id.clone();
            changed = true;
            if profiles.rooms.iter().any(|room| room.id == generated_id) {
                return Err("Zufaellige Raumprofil-ID kollidierte".into());
            }
            profiles.rooms.push(RoomProfile {
                id: generated_id.clone(),
                name: label.clone(),
                room_id: material.room_id().to_string(),
                auto_join: true,
                last_seen: None,
                status: ShareStatus::Waiting,
                members: Vec::new(),
                exports: profiles.default_direct_exports.clone(),
            });
            Ok(())
        });
        let profiles = match result {
            Ok(profiles) => profiles,
            Err(error) => return Err(cleanup_prepared_secret(error, &account)),
        };
        if selected_id != generated_id {
            finish_rebased_credential(
                &account,
                &room_secret_account(&selected_id),
                &secret,
                "Raum ist bereits mit anderem Secret gespeichert",
            )?;
        }
        let outcome = if changed {
            RoomPersistenceOutcome::Changed {
                room_profile_id: selected_id,
            }
        } else {
            RoomPersistenceOutcome::AlreadyComplete {
                room_profile_id: selected_id,
            }
        };
        Ok((profiles, outcome))
    }

    pub fn remove_direct_contact_persisted(
        default_home: Option<String>,
        contact_id: &str,
    ) -> Result<(Self, ProfileChange), String> {
        let mut changed = false;
        let profiles = Self::mutate_persisted(default_home, |profiles| {
            if profiles
                .direct_contacts
                .iter()
                .any(|contact| contact.id == contact_id)
            {
                changed = true;
                profiles
                    .direct_contacts
                    .retain(|contact| contact.id != contact_id);
                profiles
                    .direct_requests
                    .retain(|request| request.contact_id.as_deref() != Some(contact_id));
            }
            Ok(())
        })?;
        let cleanup_warning = changed
            .then(|| delete_credential_verified(&direct_contact_secret_account(contact_id)))
            .and_then(Result::err)
            .map(|error| format!("Kontakt entfernt, aber sein Secret blieb gespeichert: {error}"));
        Ok((
            profiles,
            ProfileChange {
                changed,
                cleanup_warning,
            },
        ))
    }

    pub fn remove_room_persisted(
        default_home: Option<String>,
        room_id: &str,
    ) -> Result<(Self, ProfileChange), String> {
        let mut changed = false;
        let profiles = Self::mutate_persisted(default_home, |profiles| {
            if profiles.rooms.iter().any(|room| room.id == room_id) {
                changed = true;
                profiles.rooms.retain(|room| room.id != room_id);
            }
            Ok(())
        })?;
        let cleanup_warning = changed
            .then(|| delete_credential_verified(&room_secret_account(room_id)))
            .and_then(Result::err)
            .map(|error| format!("Raum entfernt, aber sein Secret blieb gespeichert: {error}"));
        Ok((
            profiles,
            ProfileChange {
                changed,
                cleanup_warning,
            },
        ))
    }
}

fn cleanup_prepared_secret(error: String, account: &str) -> String {
    match delete_credential_verified(account) {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; neues Secret konnte nicht entfernt werden: {cleanup}"),
    }
}

fn finish_rebased_credential(
    prepared_account: &str,
    existing_account: &str,
    expected: &SecretString,
    conflict: &str,
) -> Result<(), String> {
    let matches = credential_matches(existing_account, expected);
    let cleanup = delete_credential_verified(prepared_account);
    if let Err(cleanup) = cleanup {
        return Err(format!(
            "temporaeres Secret konnte nach Rebase nicht entfernt werden: {cleanup}"
        ));
    }
    match matches {
        Ok(true) => Ok(()),
        Ok(false) => Err(conflict.to_string()),
        Err(error) => Err(format!(
            "gespeichertes Secret konnte nicht geprueft werden: {error}"
        )),
    }
}
