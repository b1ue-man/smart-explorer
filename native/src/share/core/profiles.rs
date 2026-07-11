use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::core::{hex, hex_decode, public_fingerprint, random_bytes, random_hex_token};
use super::fs::ShareExportConfig;
use super::types::{DirectContact, DirectGrant, DirectGrantState, PeerPresence, RoomProfile};

const DIRECT_CONTACT_SECRET_PREFIX: &str = "share:direct-contact:";
const ROOM_SECRET_PREFIX: &str = "share:room:";
pub(super) const SHARE_PROFILE_VERSION: u32 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ProfileRevision {
    #[default]
    Untracked,
    Missing,
    Digest([u8; 32]),
}

impl ProfileRevision {
    pub(crate) fn from_contents(contents: &str) -> Self {
        Self::Digest(Sha256::digest(contents.as_bytes()).into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareProfiles {
    #[serde(skip)]
    pub(crate) storage_revision: ProfileRevision,
    #[serde(default)]
    pub schema_version: u32,
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
            storage_revision: ProfileRevision::Untracked,
            schema_version: SHARE_PROFILE_VERSION,
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
    pub fn direct_contact_id_from_code(&self, code: &str) -> Result<Option<String>, String> {
        let parsed = DirectCode::parse(code)?;
        Ok(self
            .direct_contacts
            .iter()
            .find(|c| c.lookup_id == parsed.lookup_id)
            .map(|c| c.id.clone()))
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
            g.node_id = presence.node_id.clone();
            g.state = state;
            g.updated_at = now;
        } else {
            self.direct_grants.push(DirectGrant {
                device_id: presence.device_id.clone(),
                device_name: presence.device_name.clone(),
                public_key: presence.public_key.clone(),
                fingerprint: presence.fingerprint.clone(),
                node_id: presence.node_id.clone(),
                state,
                updated_at: now,
            });
        }
    }

    pub fn new_room_code() -> Result<String, String> {
        let room_id = random_hex_token::<12>()
            .map_err(|error| format!("Sichere Raum-ID erzeugen: {error}"))?;
        let secret = random_bytes::<32>()
            .map_err(|error| format!("Sicheres Raum-Secret erzeugen: {error}"))?;
        Ok(format!("SE-R3-{room_id}-{}", hex(&secret)))
    }
}

pub(super) fn direct_contact_secret_account(id: &str) -> String {
    format!("{DIRECT_CONTACT_SECRET_PREFIX}{id}")
}

pub(super) fn room_secret_account(id: &str) -> String {
    format!("{ROOM_SECRET_PREFIX}{id}")
}

pub(super) struct DirectCode {
    pub(super) lookup_id: String,
    pub(super) secret: Vec<u8>,
    pub(super) fingerprint: String,
    pub(super) node_id: String,
}

impl DirectCode {
    pub(super) fn parse(code: &str) -> Result<Self, String> {
        let rest = code
            .trim()
            .strip_prefix("SE-D3-")
            .ok_or_else(|| "Ungueltiger Direkt-Code".to_string())?;
        let parts: Vec<&str> = rest.rsplitn(4, '-').collect();
        if parts.len() != 4 || parts[3].trim().is_empty() {
            return Err("Ungueltiger Direkt-Code".into());
        }
        let node_id = parts[0].trim().to_string();
        let fingerprint = parts[1].to_ascii_lowercase();
        let secret = hex_decode(parts[2])?;
        if secret.len() != 32 {
            return Err("Direkt-Code enthaelt kein gueltiges Secret".into());
        }
        if fingerprint.len() < 16 || !fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Direkt-Code enthaelt keinen gueltigen Fingerprint".into());
        }
        if node_id.is_empty() {
            return Err("Direkt-Code enthaelt keine Iroh NodeId".into());
        }
        Ok(Self {
            lookup_id: parts[3].to_string(),
            secret,
            fingerprint,
            node_id,
        })
    }
}

pub(super) struct RoomCode {
    pub(super) room_id: String,
    pub(super) secret: Vec<u8>,
}

impl RoomCode {
    pub(super) fn parse(code: &str) -> Result<Self, String> {
        let rest = code
            .trim()
            .strip_prefix("SE-R3-")
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
    public_fingerprint(public_key_b64.as_bytes()) == expected.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{DirectCode, RoomCode, ShareProfiles};
    use crate::share::types::{DirectGrantState, PeerPresence};

    #[test]
    fn direct_code_parses_lookup_ids_with_dashes() {
        let secret = "00".repeat(32);
        let fp = "11".repeat(16);
        let parsed = DirectCode::parse(&format!("SE-D3-look-up-{secret}-{fp}-node123")).unwrap();
        assert_eq!(parsed.lookup_id, "look-up");
        assert_eq!(parsed.secret.len(), 32);
        assert_eq!(parsed.fingerprint, fp);
        assert_eq!(parsed.node_id, "node123");
    }

    #[test]
    fn room_code_parses_room_ids_with_dashes() {
        let secret = "22".repeat(32);
        let parsed = RoomCode::parse(&format!("SE-R3-room-id-{secret}")).unwrap();
        assert_eq!(parsed.room_id, "room-id");
        assert_eq!(parsed.secret.len(), 32);
    }

    #[test]
    fn legacy_codes_are_rejected() {
        let secret = "00".repeat(32);
        let fp = "11".repeat(16);
        assert!(DirectCode::parse(&format!("SE-D1-look-up-{secret}-{fp}")).is_err());
        assert!(RoomCode::parse(&format!("SE-R1-room-id-{secret}")).is_err());
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
            node_id: "node".into(),
            relay_url: "http://relay".into(),
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
