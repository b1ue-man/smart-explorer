use super::core::{b64, random_token};
use super::profile_persistence::ProfileChange;
use super::profiles::{
    direct_contact_secret_account, room_secret_account, DirectCode, RoomCode, ShareProfiles,
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
        save_secret_verified(&account, &b64(&parsed.secret))?;
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
            crate::creds::delete_secret_checked(&account).map_err(|cleanup| {
                format!("Direktgeraet ist bereits gespeichert; temporaeres Secret blieb: {cleanup}")
            })?;
        }
        Ok((profiles, selected_id))
    }

    pub fn add_room_from_code_persisted(
        default_home: Option<String>,
        code: &str,
        name: &str,
    ) -> Result<(Self, String), String> {
        let parsed = RoomCode::parse(code)?;
        let generated_id =
            random_token(10).map_err(|error| format!("Sichere Raumprofil-ID erzeugen: {error}"))?;
        let account = room_secret_account(&generated_id);
        save_secret_verified(&account, &b64(&parsed.secret))?;
        let label = if name.trim().is_empty() {
            "Raum".to_string()
        } else {
            name.trim().to_string()
        };
        let mut selected_id = generated_id.clone();
        let result = Self::mutate_persisted(default_home, |profiles| {
            if let Some(existing) = profiles
                .rooms
                .iter()
                .find(|room| room.room_id == parsed.room_id)
            {
                selected_id = existing.id.clone();
                return Ok(());
            }
            selected_id = generated_id.clone();
            profiles.rooms.push(RoomProfile {
                id: generated_id.clone(),
                name: label.clone(),
                room_id: parsed.room_id.clone(),
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
            crate::creds::delete_secret_checked(&account).map_err(|cleanup| {
                format!("Raum ist bereits gespeichert; temporaeres Secret blieb: {cleanup}")
            })?;
        }
        Ok((profiles, selected_id))
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
            .then(|| {
                crate::creds::delete_secret_checked(&direct_contact_secret_account(contact_id))
            })
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
            .then(|| crate::creds::delete_secret_checked(&room_secret_account(room_id)))
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

fn save_secret_verified(account: &str, secret: &str) -> Result<(), String> {
    crate::creds::set_secret(account, secret)?;
    match crate::creds::get_secret_checked(account)? {
        Some(stored) if stored == secret => Ok(()),
        Some(_) => Err("secure store returned different Share secret bytes".into()),
        None => Err("secure store did not retain the Share secret".into()),
    }
}

fn cleanup_prepared_secret(error: String, account: &str) -> String {
    match crate::creds::delete_secret_checked(account) {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; neues Secret konnte nicht entfernt werden: {cleanup}"),
    }
}
