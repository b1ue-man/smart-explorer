use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::fs::ShareExportConfig;
use super::identity::ShareIdentity;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShareScope {
    Direct { contact_id: String },
    Room { room_id: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShareStatus {
    #[default]
    Offline,
    Waiting,
    WaitingForAccess,
    Available,
    Connecting,
    Connected,
    ConnectedDirect,
    ConnectedRelay,
    Failed(String),
    IdentityConflict,
}

impl ShareStatus {
    pub fn label(&self) -> String {
        match self {
            ShareStatus::Offline => "Offline".into(),
            ShareStatus::Waiting => "Wartet".into(),
            ShareStatus::WaitingForAccess => "Warte auf Freigabe".into(),
            ShareStatus::Available => "Online".into(),
            ShareStatus::Connecting => "Verbinde".into(),
            ShareStatus::Connected => "Verbunden".into(),
            ShareStatus::ConnectedDirect => "Direkt verbunden".into(),
            ShareStatus::ConnectedRelay => "Relay verbunden".into(),
            ShareStatus::Failed(e) => format!("Fehler: {e}"),
            ShareStatus::IdentityConflict => "Identitaetskonflikt".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectAccessState {
    Pending,
    Accepted,
    Ignored,
    IdentityConflict,
}

impl DirectAccessState {
    pub fn label(&self) -> &'static str {
        match self {
            DirectAccessState::Pending => "Warte auf Freigabe",
            DirectAccessState::Accepted => "Freigegeben",
            DirectAccessState::Ignored => "Ignoriert",
            DirectAccessState::IdentityConflict => "Identitaetskonflikt",
        }
    }
}

pub(crate) fn default_direct_access_state() -> DirectAccessState {
    DirectAccessState::Accepted
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectGrantState {
    Accepted,
    Ignored,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectGrant {
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    #[serde(default)]
    pub node_id: String,
    pub state: DirectGrantState,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerPresence {
    pub kind: String,
    pub relation_id: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub relay_url: String,
    pub candidates: Vec<String>,
    pub expires_at: i64,
    pub nonce: String,
    pub proof: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectContact {
    pub id: String,
    pub display_name: String,
    pub lookup_id: String,
    pub expected_fingerprint: String,
    #[serde(default)]
    pub expected_node_id: String,
    pub remote_device_id: Option<String>,
    pub remote_public_key: Option<String>,
    pub auto_connect: bool,
    pub auto_open: bool,
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub status: ShareStatus,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub presence: Option<PeerPresence>,
    #[serde(default)]
    pub exports: ShareExportConfig,
    #[serde(default = "default_direct_access_state")]
    pub access_state: DirectAccessState,
    #[serde(default)]
    pub request_sent_at: Option<i64>,
    #[serde(default)]
    pub accepted_at: Option<i64>,
    #[serde(default)]
    pub accepted_public_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomMember {
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub public_key: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub relay_url: String,
    pub candidates: Vec<String>,
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub status: ShareStatus,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub presence: Option<PeerPresence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomProfile {
    pub id: String,
    pub name: String,
    pub room_id: String,
    pub auto_join: bool,
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub status: ShareStatus,
    #[serde(default)]
    pub members: Vec<RoomMember>,
    #[serde(default)]
    pub exports: ShareExportConfig,
}

#[derive(Clone, Debug)]
pub struct PeerEndpoint {
    pub label: String,
    pub scope: ShareScope,
    pub presence: PeerPresence,
    pub relation_secret: Vec<u8>,
    pub expected_node_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PeerOpenTarget {
    Direct { contact_id: String },
    RoomDevice { room_id: String, device_id: String },
}

impl PeerOpenTarget {
    pub fn endpoint_prefix(&self) -> String {
        match self {
            PeerOpenTarget::Direct { contact_id } => format!("share://direct/{contact_id}"),
            PeerOpenTarget::RoomDevice { room_id, device_id } => {
                format!("share://room/{room_id}/{device_id}")
            }
        }
    }

    pub fn from_endpoint(endpoint: &str) -> Option<(Self, String)> {
        let rest = endpoint.trim().strip_prefix("share://")?;
        if let Some(rest) = rest.strip_prefix("direct/") {
            let mut parts = rest.splitn(2, '/');
            let contact_id = parts.next()?.trim();
            if contact_id.is_empty() {
                return None;
            }
            let path = parts
                .next()
                .map(|p| format!("/{}", p.trim_start_matches('/')))
                .unwrap_or_else(|| "/".to_string());
            return Some((
                PeerOpenTarget::Direct {
                    contact_id: contact_id.to_string(),
                },
                normalize_endpoint_path(&path),
            ));
        }
        if let Some(rest) = rest.strip_prefix("room/") {
            let mut parts = rest.splitn(3, '/');
            let room_id = parts.next()?.trim();
            let device_id = parts.next()?.trim();
            if room_id.is_empty() || device_id.is_empty() {
                return None;
            }
            let path = parts
                .next()
                .map(|p| format!("/{}", p.trim_start_matches('/')))
                .unwrap_or_else(|| "/".to_string());
            return Some((
                PeerOpenTarget::RoomDevice {
                    room_id: room_id.to_string(),
                    device_id: device_id.to_string(),
                },
                normalize_endpoint_path(&path),
            ));
        }
        None
    }
}

fn normalize_endpoint_path(path: &str) -> String {
    let p = path.trim().replace('\\', "/");
    if p.is_empty() {
        "/".to_string()
    } else if p.starts_with('/') {
        p
    } else {
        format!("/{p}")
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::PeerOpenTarget;

    #[test]
    fn direct_endpoint_round_trips_with_path() {
        let target = PeerOpenTarget::Direct {
            contact_id: "contact-a".into(),
        };
        assert_eq!(target.endpoint_prefix(), "share://direct/contact-a");
        let (parsed, root) =
            PeerOpenTarget::from_endpoint("share://direct/contact-a/Gate/Sub").unwrap();
        assert_eq!(parsed, target);
        assert_eq!(root, "/Gate/Sub");
    }

    #[test]
    fn room_endpoint_round_trips_with_path() {
        let target = PeerOpenTarget::RoomDevice {
            room_id: "room-a".into(),
            device_id: "device-b".into(),
        };
        assert_eq!(target.endpoint_prefix(), "share://room/room-a/device-b");
        let (parsed, root) =
            PeerOpenTarget::from_endpoint("share://room/room-a/device-b/Docs").unwrap();
        assert_eq!(parsed, target);
        assert_eq!(root, "/Docs");
    }
}

/// What the UI tells the share worker to do.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ShareCmd {
    Configure {
        direct: Vec<DirectContact>,
        direct_grants: Vec<DirectGrant>,
        rooms: Vec<RoomProfile>,
        default_direct_exports: ShareExportConfig,
    },
    Refresh,
    Stop,
    SetDirectOnline {
        online: bool,
    },
    LeaveRoom {
        room_id: String,
    },
    RequestDirect {
        contact_id: String,
    },
    AnswerDirectRequest {
        lookup_id: String,
        presence: PeerPresence,
        accepted: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ShareEvent {
    Status(String),
    Error(String),
    ServerConnected,
    ServerDisconnected(String),
    DirectAvailable {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectOffline {
        lookup_id: String,
    },
    DirectAccessRequest {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectAccessAccepted {
        lookup_id: String,
        requester_device_id: String,
        accepted: bool,
        presence: Option<PeerPresence>,
        msg: Option<String>,
    },
    RoomRoster {
        room_id: String,
        members: Vec<PeerPresence>,
    },
    RoomJoined {
        room_id: String,
        presence: PeerPresence,
    },
    RoomLeft {
        room_id: String,
        device_id: String,
    },
}

pub(crate) type CmdTx = Sender<ShareCmd>;

#[derive(Clone)]
pub(crate) struct ShareAuthState {
    pub(crate) identity: ShareIdentity,
    pub(crate) direct_secret: Vec<u8>,
    pub(crate) default_direct_exports: ShareExportConfig,
    pub(crate) direct_contacts: Vec<DirectContact>,
    pub(crate) direct_grants: Vec<DirectGrant>,
    pub(crate) rooms: Vec<RoomProfile>,
    pub(crate) seen_nonces: HashSet<String>,
    pub(crate) direct_online: bool,
}
