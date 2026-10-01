use serde::{Deserialize, Serialize};

use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::types::{ExecRequest, ExecResult, PeerPresence};

#[path = "batch_wire.rs"]
mod batch_wire;

pub(crate) use self::batch_wire::{
    discardable_stage, plan_batches, validate_get, validate_put, BatchPart, FsBatchGet,
    FsBatchOutcome, FsBatchPut, FsBatchStatus, FsTransferCapabilities, BATCH_MAX_BYTES,
    BATCH_MAX_FILES, NONCE_HEX_LEN, TRANSFER_V1_CAPABILITY,
};

pub(crate) const TRACKED_DIRECT_CAPABILITY: &str = "tracked_direct_v1";
/// Server keeps idle clients alive with its own keepalives (V1).
pub(crate) const IDLE_KEEPALIVE_CAPABILITY: &str = "idle_keepalive_v1";
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
    /// Only after `idle_keepalive_v1` was negotiated. `keepalive_secs`
    /// proposes a shorter server keepalive (proxies with short timeouts).
    SetIdle {
        idle: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keepalive_secs: Option<u32>,
    },
    KeepaliveAck,
}

/// Idle-mode messages of a server that negotiated `idle_keepalive_v1`.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum IdleServerMsg {
    IdleAck {
        idle: bool,
        #[serde(default)]
        keepalive_secs: Option<u32>,
    },
    Keepalive,
}

impl IdleServerMsg {
    /// `Ok(None)` for every other server message.
    pub(crate) fn parse(line: &str) -> Result<Option<Self>, String> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| error.to_string())?;
        if !matches!(
            value.get("t").and_then(serde_json::Value::as_str),
            Some("idle_ack" | "keepalive")
        ) {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| error.to_string())
    }
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
    /// Additive: the host's transfer features, the same for every path.
    /// Hosts before transfer v1 omit it; their clients ignore it.
    #[serde(default, skip_serializing_if = "FsTransferCapabilities::is_absent")]
    pub(crate) transfer: FsTransferCapabilities,
}

impl From<crate::vfs::StagedWriteCapabilities> for FsWriteCapabilities {
    fn from(value: crate::vfs::StagedWriteCapabilities) -> Self {
        Self {
            create: value.create,
            replace: value.replace,
            namespace_replace: value.namespace_replace,
            transfer: FsTransferCapabilities::default(),
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
    AlreadyExists,
    Unsupported,
    /// The host is full or its backend reported a rate limit: congestion,
    /// not failure. Older peers read it as `Unknown`.
    Busy,
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
    StorageAnalysis {
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
    /// Transfer v1: many small new files in one stream. Each entry lands in a
    /// private stage named with `nonce`; nothing is published before the
    /// client's WriteDone, and nothing is ever replaced.
    PutBatch {
        nonce: String,
        entries: Vec<FsBatchPut>,
    },
    /// Transfer v1: the state of this principal's PutBatch `nonce` after its
    /// reply was lost.
    PutBatchStatus {
        nonce: String,
    },
    /// Transfer v1: many small files in one stream, in request order.
    GetBatch {
        items: Vec<FsBatchGet>,
    },
    /// Transfer v1: read from `offset` (resume) or by provider ID.
    ReadAt {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        offset: u64,
    },
    /// Transfer v1: one directory level; `exclusive` fails on a taken name.
    CreateDir {
        path: String,
        #[serde(default, skip_serializing_if = "is_false")]
        exclusive: bool,
    },
    /// Transfer v1: publish a stage only if `destination` is absent; the
    /// host validates the stage itself. `copy` selects the copy-stage commit.
    PromoteNoReplace {
        staged: String,
        destination: String,
        #[serde(default, skip_serializing_if = "is_false")]
        copy: bool,
    },
    /// Transfer v1: remove a copy stage the client created and never published.
    DiscardStage {
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
                | Self::CreateDir { .. }
                | Self::Rename { .. }
                | Self::RenameNoReplace { .. }
                | Self::PromoteStaged { .. }
                | Self::PromoteNoReplace { .. }
                | Self::CopyFile { .. }
                | Self::RemoveFile { .. }
                | Self::RemoveDir { .. }
                | Self::DiscardStage { .. }
                | Self::PutBatch { .. }
        )
    }

    /// Batches admit every entry again, exactly like one single request.
    pub(super) fn is_batch(&self) -> bool {
        matches!(self, Self::PutBatch { .. } | Self::GetBatch { .. })
    }

    /// Requests a host before transfer v1 cannot parse; clients send them
    /// only after that host's Capabilities advertised v1.
    pub(super) fn is_transfer_v1(&self) -> bool {
        matches!(
            self,
            Self::PutBatch { .. }
                | Self::PutBatchStatus { .. }
                | Self::GetBatch { .. }
                | Self::ReadAt { .. }
                | Self::CreateDir { .. }
                | Self::PromoteNoReplace { .. }
                | Self::DiscardStage { .. }
        )
    }
}

pub(crate) use super::fs_response::FsResponse;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "c", rename_all = "snake_case")]
pub(crate) enum Ctrl {
    PeerHello {
        hello: PeerHello,
    },
    PeerHelloOk,
    /// Unit-only selector for the bounded binary reciprocal Direct stream.
    /// Relation material is never serialized into this JSON control frame.
    DirectReciprocal,
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
#[path = "wire_tests.rs"]
mod remote_drive_task_wire_tests;
