use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

use super::direct_relation::{apply_contact_runtime, DirectContactRuntime};
use super::exec_policy::ExecGrant;
use super::fs::ShareExportConfig;
use super::identity::ShareIdentity;
use super::profiles::ShareProfiles;
use super::room_relation::{apply_room_runtime, RoomRuntime};
use super::{direct_ledger::DirectRequestEntry, direct_signal_event::DirectSignalEvent};

// The relation records live in their own modules (V5); their former paths stay.
pub use super::direct_relation::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, DirectRelationFlags,
    DirectRequestPolicy, PresenceApply,
};
pub use super::room_relation::{MemberUpsert, RoomMemberAdmission, RoomMemberFlags, RoomPolicy};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShareScope {
    Direct { contact_id: String },
    Room { room_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecRequest {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    pub shell: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
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

pub(super) const MAX_PRESENCE_FUTURE_SECS: i64 = 15 * 60;

impl PeerPresence {
    /// A persisted presence is only routing evidence while its signed lifetime
    /// is current. Identity pins and relation grants outlive this short-lived
    /// network snapshot and are intentionally kept separately.
    pub fn is_current_at(&self, now: i64) -> bool {
        self.expires_at >= now && self.expires_at <= now.saturating_add(MAX_PRESENCE_FUTURE_SECS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Denies the member everywhere: blocked by the user, or pending
    /// admission (`relation.admission == Pending` implies `blocked`).
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub exec: ExecGrant,
    #[serde(default)]
    pub presence: Option<PeerPresence>,
    #[serde(default)]
    pub relation: RoomMemberFlags,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    #[serde(default = "RoomPolicy::legacy")]
    pub policy: RoomPolicy,
}

#[derive(Clone)]
pub struct PeerEndpoint {
    pub label: String,
    pub scope: ShareScope,
    pub presence: PeerPresence,
    pub relation_secret: Vec<u8>,
    pub expected_node_id: Option<String>,
}

/// Exact, display-name-independent identity addressed by an Exec grant.
///
/// Unlike file-open targets, direct grants do not require a saved contact and
/// room grants never accept a local profile id. Every mutation repeats all
/// cryptographic pins so stale UI labels or replaced devices fail closed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecGrantTarget {
    Direct {
        device_id: String,
        public_key: String,
        fingerprint: String,
        node_id: String,
    },
    RoomMember {
        room_id: String,
        device_id: String,
        public_key: String,
        fingerprint: String,
        node_id: String,
    },
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
        let rest = endpoint.strip_prefix("share://")?;
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
    let p = path.replace('\\', "/");
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
    ConfigureProfiles {
        profiles: Box<ShareProfiles>,
    },
    Configure {
        direct: Vec<DirectContact>,
        direct_grants: Vec<DirectGrant>,
        rooms: Vec<RoomProfile>,
        default_direct_exports: ShareExportConfig,
    },
    SyncDirectRequests {
        direct_requests: Vec<DirectRequestEntry>,
        direct_request_tombstones: Vec<super::direct_request_tombstone::DirectRequestTombstone>,
    },
    Refresh,
    Stop,
    SetDirectOnline {
        online: bool,
    },
    EnableExec {
        target: ExecGrantTarget,
    },
    DisableExec {
        target: ExecGrantTarget,
    },
    ApplyExecGrant {
        target: ExecGrantTarget,
        principal: Box<super::exec_types::ExecPrincipal>,
        policy: ExecGrant,
    },
    LeaveRoom {
        room_id: String,
    },
    RequestDirect {
        contact_id: String,
    },
    AnswerLegacyDirectRequest {
        selector: String,
        decision_revision: u64,
        lookup_id: String,
        requester_device_id: String,
        accepted: bool,
    },
    Discovery(super::discovery_signal_types::DiscoveryCommand),
    /// FA3: daemon-internal runtime data (presence, LAN routes, status, newly
    /// seen room members). No configuration transition, no invalidation;
    /// IPC clients may not send it.
    UpdateRuntime {
        runtime: Box<RelationRuntime>,
    },
}

/// Payload of `ShareCmd::UpdateRuntime`: the runtime half of every contact
/// and room. It never changes grants, pins, access states, exports, room
/// policy or a known member's rights.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationRuntime {
    pub contacts: Vec<DirectContactRuntime>,
    pub rooms: Vec<RoomRuntime>,
}

impl RelationRuntime {
    pub fn from_profiles(profiles: &ShareProfiles) -> Self {
        Self {
            contacts: profiles
                .direct_contacts
                .iter()
                .map(DirectContactRuntime::of)
                .collect(),
            rooms: profiles.rooms.iter().map(RoomRuntime::of).collect(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ShareCmdResult {
    Applied,
    ExecGrant(Box<super::exec_grant_runtime::ExecGrantMutation>),
    DiscoveryOffer(super::discovery_signal_types::DiscoveryOfferHandle),
    DiscoveryExchange(super::discovery_signal_types::DiscoveryExchangeHandle),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ShareEvent {
    Status(String),
    Error(String),
    ServerConnected,
    ServerDisconnected(String),
    DirectSignal(DirectSignalEvent),
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
    Discovery(super::discovery_signal_types::DiscoveryEvent),
    /// Secret-free notification emitted only after a persisted reciprocal or
    /// discovery exchange has completed its application-level acknowledgement.
    RuntimeProfilesCommitted,
    LanPeerSeen {
        contact_id: String,
        candidates: Vec<String>,
        uplink: bool,
    },
    LanPeerLost {
        contact_id: String,
    },
}

pub(crate) struct PendingShareCmd {
    pub(crate) command: ShareCmd,
    pub(crate) acknowledgement: Sender<Result<ShareCmdResult, String>>,
    pub(crate) expires_at: Instant,
}

pub(crate) type CmdTx = Sender<PendingShareCmd>;

#[derive(Clone)]
pub(crate) struct ShareAuthState {
    pub(crate) identity: ShareIdentity,
    pub(crate) direct_secret: Vec<u8>,
    pub(crate) default_direct_exports: ShareExportConfig,
    pub(crate) direct_contacts: Vec<DirectContact>,
    pub(crate) direct_grants: Vec<DirectGrant>,
    pub(crate) rooms: Vec<RoomProfile>,
    pub(crate) direct_requests: Vec<DirectRequestEntry>,
    pub(crate) direct_request_tombstones:
        Vec<super::direct_request_tombstone::DirectRequestTombstone>,
    pub(crate) seen_nonces: HashSet<String>,
    pub(crate) direct_online: bool,
    pub(crate) authorization_epoch: u64,
}

impl std::fmt::Debug for ShareAuthState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShareAuthState")
            .field("identity", &"[REDACTED]")
            .field("direct_secret", &"[REDACTED]")
            .field("direct_contact_count", &self.direct_contacts.len())
            .field("direct_grant_count", &self.direct_grants.len())
            .field("room_count", &self.rooms.len())
            .field("direct_request_count", &self.direct_requests.len())
            .field(
                "direct_request_tombstone_count",
                &self.direct_request_tombstones.len(),
            )
            .field("seen_nonce_count", &self.seen_nonces.len())
            .field("direct_online", &self.direct_online)
            .field("authorization_epoch", &self.authorization_epoch)
            .finish_non_exhaustive()
    }
}

impl ShareAuthState {
    /// Applies `ShareCmd::UpdateRuntime` (FA3). Returns whether anything
    /// changed; the authorization epoch stays.
    pub(crate) fn apply_runtime(&mut self, runtime: &RelationRuntime) -> bool {
        let contacts = apply_contact_runtime(&mut self.direct_contacts, &runtime.contacts);
        let rooms = apply_room_runtime(&mut self.rooms, &runtime.rooms);
        contacts || rooms
    }
}

impl Drop for ShareAuthState {
    fn drop(&mut self) {
        self.direct_secret.fill(0);
    }
}
