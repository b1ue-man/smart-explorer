use serde::{Deserialize, Serialize};

use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::types::{ExecRequest, ExecResult, PeerPresence};

pub(crate) const TRACKED_DIRECT_CAPABILITY: &str = "tracked_direct_v1";
pub(crate) const MOUNT_PATH_CAPABILITY_CONTRACT_VERSION: u8 = 1;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum ClientMsg {
    Hello {
        protocol_version: u32,
        device_id: String,
        device_name: String,
        listen_port: u16,
        lan: Vec<String>,
        public_key: String,
        fingerprint: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    PublishDirect {
        presence: PeerPresence,
    },
    UnpublishDirect {
        lookup_id: String,
    },
    WatchDirect {
        lookup_id: String,
    },
    RequestDirect {
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
    UnwatchDirect {
        lookup_id: String,
    },
    JoinRoom {
        room_id: String,
        presence: PeerPresence,
    },
    LeaveRoom {
        room_id: String,
    },
    Heartbeat,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum SrvMsg {
    HelloOk {
        #[serde(default)]
        capabilities: Vec<String>,
    },
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
    Error {
        scope: String,
        msg: String,
    },
    Pong,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum TrackedDirectClientMsg {
    #[serde(rename = "submit_direct_request")]
    Request {
        request: Box<SignedDirectRequest>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        legacy_presence: Option<PeerPresence>,
    },
    #[serde(rename = "submit_direct_request_receipt")]
    RequestReceipt { receipt: SignedDirectRequestReceipt },
    #[serde(rename = "submit_direct_decision")]
    Decision { decision: SignedDirectDecision },
    #[serde(rename = "submit_direct_decision_receipt")]
    DecisionReceipt {
        receipt: SignedDirectDecisionReceipt,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum TrackedDirectServerMsg {
    #[serde(rename = "direct_request")]
    Request { request: SignedDirectRequest },
    #[serde(rename = "direct_request_receipt")]
    RequestReceipt { receipt: SignedDirectRequestReceipt },
    #[serde(rename = "direct_decision")]
    Decision { decision: SignedDirectDecision },
    #[serde(rename = "direct_decision_receipt")]
    DecisionReceipt {
        receipt: SignedDirectDecisionReceipt,
    },
    #[serde(rename = "direct_route_ack")]
    RouteAck {
        request_id: DirectRequestId,
        route: DirectRoute,
        outcome: DirectRouteOutcome,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DirectRoute {
    Request,
    RequestReceipt,
    Decision,
    DecisionReceipt,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DirectRouteOutcome {
    Forwarded,
    LegacyForwarded,
    TargetOffline,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct PeerHello {
    pub(crate) protocol_version: u32,
    pub(crate) relation_kind: String,
    pub(crate) relation_id: String,
    pub(crate) device_id: String,
    pub(crate) public_key: String,
    #[serde(default)]
    pub(crate) node_id: String,
    #[serde(default)]
    pub(crate) session_nonce: String,
    #[serde(default)]
    pub(crate) session_proof: String,
    pub(crate) requested_capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FsMeta {
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) is_symlink: bool,
    pub(crate) size: u64,
    pub(crate) mtime_ms: i64,
    pub(crate) btime_ms: i64,
    pub(crate) hidden: bool,
    pub(crate) system: bool,
    pub(crate) id: Option<String>,
}

/// One compact node in the bounded, post-order tree-walk stream. IDs are
/// assigned parent-first, while nodes are emitted child-first so the receiver
/// can assemble the final tree without retaining a second flat copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsWalkNode {
    pub(crate) id: u64,
    pub(crate) parent: Option<u64>,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsWriteCapabilities {
    pub(crate) create: bool,
    pub(crate) replace: bool,
    pub(crate) namespace_replace: bool,
}

impl From<crate::vfs::StagedWriteCapabilities> for FsWriteCapabilities {
    fn from(value: crate::vfs::StagedWriteCapabilities) -> Self {
        Self {
            create: value.create,
            replace: value.replace,
            namespace_replace: value.namespace_replace,
        }
    }
}

impl From<FsWriteCapabilities> for crate::vfs::StagedWriteCapabilities {
    fn from(value: FsWriteCapabilities) -> Self {
        Self {
            create: value.create,
            replace: value.replace,
            namespace_replace: value.namespace_replace,
        }
    }
}

/// A deliberately small, additive classification for filesystem failures.
/// The message remains the source of detail; this kind exists only where a
/// caller must make a safe control-flow decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsErrorKind {
    NotFound,
    PermissionDenied,
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum FsRequest {
    Capabilities {
        path: String,
        /// Mount hosts request a principal-bound root lease. Browsing and UI
        /// probes leave this false so a capability inspection cannot consume
        /// the server's bounded lease table.
        #[serde(default, skip_serializing_if = "is_false")]
        acquire_lease: bool,
        /// Stable for this mount acquisition and all of its safe retries.
        /// Distinct mounted backends use distinct IDs so release ownership is
        /// never inferred from an otherwise identical root binding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease_request_id: Option<String>,
    },
    ReleaseLease,
    ListDir {
        path: String,
    },
    Stat {
        path: String,
    },
    WalkTree {
        path: String,
    },
    StorageSnapshot {
        path: String,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
    },
    WriteNew {
        path: String,
    },
    WriteDone,
    MkdirAll {
        path: String,
    },
    Rename {
        src: String,
        dst: String,
    },
    RenameNoReplace {
        src: String,
        dst: String,
    },
    PromoteStaged {
        staged: String,
        destination: String,
    },
    CopyFile {
        src: String,
        dst: String,
    },
    RemoveFile {
        path: String,
    },
    RemoveDir {
        path: String,
    },
}

impl FsRequest {
    pub(super) fn mutates_filesystem(&self) -> bool {
        matches!(
            self,
            Self::Write { .. }
                | Self::WriteNew { .. }
                | Self::MkdirAll { .. }
                | Self::Rename { .. }
                | Self::RenameNoReplace { .. }
                | Self::PromoteStaged { .. }
                | Self::CopyFile { .. }
                | Self::RemoveFile { .. }
                | Self::RemoveDir { .. }
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "r", rename_all = "snake_case")]
pub(crate) enum FsResponse {
    Capabilities {
        capabilities: FsWriteCapabilities,
        /// Additive protocol-v3 fields. Legacy peers omit them and therefore
        /// deserialize to contract zero, no lease, and an unconfined root.
        #[serde(default)]
        contract_version: u8,
        #[serde(default)]
        root_confined: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease: Option<String>,
        /// Additive advertisement. Absent means the peer only supports the
        /// legacy WalkTree stream.
        #[serde(default)]
        storage_snapshot_v1: bool,
    },
    Entries {
        entries: Vec<FsMeta>,
    },
    Meta {
        meta: FsMeta,
    },
    WalkBatch {
        nodes: Vec<FsWalkNode>,
        files: u64,
        dirs: u64,
        bytes: u64,
    },
    WalkDone {
        files: u64,
        dirs: u64,
        bytes: u64,
        nodes: u64,
    },
    SnapshotProgress {
        files: u64,
        dirs: u64,
        bytes: u64,
        nodes: u64,
    },
    SnapshotReady {
        encoded_len: u64,
        sha256: [u8; 32],
        files: u64,
        dirs: u64,
        bytes: u64,
        nodes: u64,
    },
    SnapshotDone {
        files: u64,
        dirs: u64,
        bytes: u64,
        nodes: u64,
    },
    Data {
        size: u64,
    },
    Ready,
    Ok,
    Err {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<FsErrorKind>,
        msg: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "c", rename_all = "snake_case")]
pub(crate) enum Ctrl {
    PeerHello {
        hello: PeerHello,
    },
    PeerHelloOk,
    Ping {
        nonce: String,
    },
    Pong {
        nonce: String,
    },
    Fs {
        req: FsRequest,
        /// Opaque mount-root lease, scoped by the server to the authenticated
        /// peer principal. Older clients omit it and retain stateless browsing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease: Option<String>,
    },
    FsResp {
        resp: FsResponse,
    },
    Exec {
        req: ExecRequest,
    },
    ExecResp {
        result: ExecResult,
    },
    ExecErr {
        msg: String,
    },
}

#[cfg(test)]
mod remote_drive_task_wire_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case")]
    enum LegacyRequest {
        Capabilities {
            path: String,
            #[serde(default)]
            acquire_lease: bool,
        },
    }

    #[test]
    fn remote_drive_task_mount_request_id_is_additive_for_legacy_peer() {
        let request = FsRequest::Capabilities {
            path: "/Docs".into(),
            acquire_lease: true,
            lease_request_id: Some("request-a".into()),
        };
        let json = serde_json::to_string(&request).unwrap();
        let LegacyRequest::Capabilities {
            path,
            acquire_lease,
        } = serde_json::from_str(&json).unwrap();
        assert_eq!(path, "/Docs");
        assert!(acquire_lease);

        let decoded: FsRequest =
            serde_json::from_str(r#"{"op":"capabilities","path":"/Docs","acquire_lease":true}"#)
                .unwrap();
        assert!(matches!(
            decoded,
            FsRequest::Capabilities {
                lease_request_id: None,
                ..
            }
        ));
    }

    #[test]
    fn remote_drive_task_legacy_peer_rejects_unknown_release_without_reinterpreting_it() {
        let json = serde_json::to_string(&FsRequest::ReleaseLease).unwrap();
        assert!(serde_json::from_str::<LegacyRequest>(&json).is_err());
    }
}
