use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};

use super::fs::ShareExportConfig;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShareScope {
    Direct { contact_id: String },
    Room { room_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShareStatus {
    Offline,
    Waiting,
    WaitingForAccess,
    Available,
    Connecting,
    Connected,
    Failed(String),
    IdentityConflict,
}

impl Default for ShareStatus {
    fn default() -> Self {
        ShareStatus::Offline
    }
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
    pub expected_public_key: Option<Vec<u8>>,
    pub server: String,
}

#[derive(Clone, Debug)]
pub enum PeerOpenTarget {
    Direct { contact_id: String },
    RoomDevice { room_id: String, device_id: String },
}

/// What the UI tells the share worker to do.
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
    #[allow(dead_code)]
    Send(Vec<String>),
    #[allow(dead_code)]
    Answer {
        id: u64,
        accept: bool,
    },
}

#[derive(Clone, Debug)]
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
    #[allow(dead_code)]
    Incoming {
        id: u64,
        from: String,
        files: Vec<(String, u64)>,
    },
    #[allow(dead_code)]
    Progress {
        done: u64,
        total: u64,
    },
    #[allow(dead_code)]
    Received {
        count: usize,
        dir: String,
    },
    #[allow(dead_code)]
    Sent {
        count: usize,
    },
}

pub(crate) type CmdTx = Sender<ShareCmd>;
