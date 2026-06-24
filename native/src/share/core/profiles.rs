use serde::{Deserialize, Serialize};

use super::core::{
    b64, b64_decode, hex, hex_decode, public_fingerprint, random_bytes, random_hex_token,
    random_token,
};
use super::fs::{ShareExportConfig, SharedRoot};
use super::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerPresence, RoomProfile,
    ShareStatus,
};

const PROFILES_FILE: &str = "share_profiles.json";
const DIRECT_CONTACT_SECRET_PREFIX: &str = "share:direct-contact:";
const ROOM_SECRET_PREFIX: &str = "share:room:";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareProfiles {
    #[serde(default = "default_true")]
    pub auto_connect: bool,
    #[serde(default)]
    pub default_direct_exports: ShareExportConfig,
    #[serde(default)]
    pub direct_contacts: Vec<DirectContact>,
    #[serde(default)]
    pub direct_grants: Vec<DirectGrant>,
    #[serde(default)]
    pub rooms: Vec<RoomProfile>,
}

impl Default for ShareProfiles {
    fn default() -> Self {
        Self {
            auto_connect: true,
            default_direct_exports: ShareExportConfig::default(),
            direct_contacts: Vec::new(),
            direct_grants: Vec::new(),
            rooms: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

impl ShareProfiles {
    pub fn load(default_home: Option<String>) -> Self {
        let path = crate::support_dirs::app_data_file(PROFILES_FILE);
        let mut profiles = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<ShareProfiles>(&s).ok())
            .unwrap_or_default();
        if profiles.default_direct_exports.roots.is_empty() {
            if let Some(home) = default_home {
                profiles.default_direct_exports.roots.push(SharedRoot {
                    label: "Home".to_string(),
                    path: home,
                });
            }
        }
        profiles
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = crate::support_dirs::app_data_file(PROFILES_FILE);
        std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))?;
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }

    pub fn add_direct_from_code(&mut self, code: &str, name: &str) -> Result<String, String> {
        let parsed = DirectCode::parse(code)?;
        if self
            .direct_contacts
            .iter()
            .any(|c| c.lookup_id == parsed.lookup_id)
        {
            return Err("Direktgeraet ist bereits gespeichert".into());
        }
        let id = random_token(10);
        let label = if name.trim().is_empty() {
            format!(
                "Direkt {}",
                &parsed.fingerprint[..parsed.fingerprint.len().min(8)]
            )
        } else {
            name.trim().to_string()
        };
        crate::creds::set_secret(&direct_contact_secret_account(&id), &b64(&parsed.secret))
            .map_err(|e| format!("Secret speichern: {e}"))?;
        self.direct_contacts.push(DirectContact {
            id: id.clone(),
            display_name: label,
            lookup_id: parsed.lookup_id,
            expected_fingerprint: parsed.fingerprint,
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::WaitingForAccess,
            last_error: None,
            presence: None,
            exports: self.default_direct_exports.clone(),
            access_state: DirectAccessState::Pending,
            request_sent_at: Some(super::core::now_secs()),
            accepted_at: None,
            accepted_public_key: None,
        });
        let _ = self.save();
        Ok(id)
    }

    pub fn grant_for(&self, device_id: &str) -> Option<&DirectGrant> {
        self.direct_grants.iter().find(|g| g.device_id == device_id)
    }

    pub fn set_direct_grant(&mut self, presence: &PeerPresence, state: DirectGrantState) {
        let now = super::core::now_secs();
        if let Some(g) = self
            .direct_grants
            .iter_mut()
            .find(|g| g.device_id == presence.device_id)
        {
            g.device_name = presence.device_name.clone();
            g.public_key = presence.public_key.clone();
            g.fingerprint = presence.fingerprint.clone();
            g.state = state;
            g.updated_at = now;
        } else {
            self.direct_grants.push(DirectGrant {
                device_id: presence.device_id.clone(),
                device_name: presence.device_name.clone(),
                public_key: presence.public_key.clone(),
                fingerprint: presence.fingerprint.clone(),
                state,
                updated_at: now,
            });
        }
    }

    pub fn add_room_from_code(&mut self, code: &str, name: &str) -> Result<String, String> {
        let parsed = RoomCode::parse(code)?;
        if let Some(existing) = self.rooms.iter().find(|r| r.room_id == parsed.room_id) {
            return Ok(existing.id.clone());
        }
        let id = random_token(10);
        let label = if name.trim().is_empty() {
            "Raum".to_string()
        } else {
            name.trim().to_string()
        };
        crate::creds::set_secret(&room_secret_account(&id), &b64(&parsed.secret))
            .map_err(|e| format!("Raum-Secret speichern: {e}"))?;
        self.rooms.push(RoomProfile {
            id: id.clone(),
            name: label,
            room_id: parsed.room_id,
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: Vec::new(),
            exports: self.default_direct_exports.clone(),
        });
        let _ = self.save();
        Ok(id)
    }

    pub fn new_room_code() -> String {
        let room_id = random_hex_token::<12>();
        let secret = random_bytes::<32>();
        format!("SE-R1-{room_id}-{}", hex(&secret))
    }

    pub fn direct_secret(contact: &DirectContact) -> Option<Vec<u8>> {
        crate::creds::get_secret(&direct_contact_secret_account(&contact.id))
            .and_then(|s| b64_decode(&s).ok())
    }

    pub fn room_secret(room: &RoomProfile) -> Option<Vec<u8>> {
        crate::creds::get_secret(&room_secret_account(&room.id)).and_then(|s| b64_decode(&s).ok())
    }

    pub fn room_code(room: &RoomProfile) -> Option<String> {
        Self::room_secret(room).map(|s| format!("SE-R1-{}-{}", room.room_id, hex(&s)))
    }
}

pub(crate) fn direct_contact_secret_account(id: &str) -> String {
    format!("{DIRECT_CONTACT_SECRET_PREFIX}{id}")
}

pub(crate) fn room_secret_account(id: &str) -> String {
    format!("{ROOM_SECRET_PREFIX}{id}")
}

struct DirectCode {
    lookup_id: String,
    secret: Vec<u8>,
    fingerprint: String,
}

impl DirectCode {
    fn parse(code: &str) -> Result<Self, String> {
        let rest = code
            .trim()
            .strip_prefix("SE-D1-")
            .ok_or_else(|| "Ungueltiger Direkt-Code".to_string())?;
        let parts: Vec<&str> = rest.rsplitn(3, '-').collect();
        if parts.len() != 3 || parts[2].trim().is_empty() {
            return Err("Ungueltiger Direkt-Code".into());
        }
        let fingerprint = parts[0].to_ascii_lowercase();
        let secret = hex_decode(parts[1])?;
        if secret.len() != 32 {
            return Err("Direkt-Code enthaelt kein gueltiges Secret".into());
        }
        if fingerprint.len() < 16 || !fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Direkt-Code enthaelt keinen gueltigen Fingerprint".into());
        }
        Ok(Self {
            lookup_id: parts[2].to_string(),
            secret,
            fingerprint,
        })
    }
}

struct RoomCode {
    room_id: String,
    secret: Vec<u8>,
}

impl RoomCode {
    fn parse(code: &str) -> Result<Self, String> {
        let rest = code
            .trim()
            .strip_prefix("SE-R1-")
            .ok_or_else(|| "Ungueltiger Raum-Code".to_string())?;
        let parts: Vec<&str> = rest.rsplitn(2, '-').collect();
        if parts.len() != 2 || parts[1].trim().is_empty() {
            return Err("Ungueltiger Raum-Code".into());
        }
        let secret = hex_decode(parts[0])?;
        if secret.len() != 32 {
            return Err("Raum-Code enthaelt kein gueltiges Secret".into());
        }
        Ok(Self {
            room_id: parts[1].to_string(),
            secret,
        })
    }
}

pub(crate) fn fingerprint_matches(public_key_b64: &str, expected: &str) -> bool {
    b64_decode(public_key_b64)
        .map(|pk| public_fingerprint(&pk) == expected.to_ascii_lowercase())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{DirectCode, RoomCode, ShareProfiles};
    use crate::share::types::{DirectGrantState, PeerPresence};

    #[test]
    fn direct_code_parses_lookup_ids_with_dashes() {
        let secret = "00".repeat(32);
        let fp = "11".repeat(16);
        let parsed = DirectCode::parse(&format!("SE-D1-look-up-{secret}-{fp}")).unwrap();
        assert_eq!(parsed.lookup_id, "look-up");
        assert_eq!(parsed.secret.len(), 32);
        assert_eq!(parsed.fingerprint, fp);
    }

    #[test]
    fn room_code_parses_room_ids_with_dashes() {
        let secret = "22".repeat(32);
        let parsed = RoomCode::parse(&format!("SE-R1-room-id-{secret}")).unwrap();
        assert_eq!(parsed.room_id, "room-id");
        assert_eq!(parsed.secret.len(), 32);
    }

    #[test]
    fn direct_grant_upsert_persists_state_by_device() {
        let mut profiles = ShareProfiles::default();
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: "lookup".into(),
            device_id: "dev-a".into(),
            device_name: "Device A".into(),
            public_key: "pk".into(),
            fingerprint: "fp".into(),
            candidates: Vec::new(),
            expires_at: 1,
            nonce: "n".into(),
            proof: "proof".into(),
        };
        profiles.set_direct_grant(&presence, DirectGrantState::Accepted);
        assert_eq!(profiles.direct_grants.len(), 1);
        assert_eq!(
            profiles.grant_for("dev-a").unwrap().state,
            DirectGrantState::Accepted
        );
        profiles.set_direct_grant(&presence, DirectGrantState::Ignored);
        assert_eq!(profiles.direct_grants.len(), 1);
        assert_eq!(
            profiles.grant_for("dev-a").unwrap().state,
            DirectGrantState::Ignored
        );
    }
}
