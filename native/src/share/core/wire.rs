use serde::{Deserialize, Serialize};

use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::export_config::ExportAccess;
use super::types::{ExecRequest, ExecResult, PeerPresence};

#[path = "batch_wire.rs"]
mod batch_wire;
#[path = "fs_request.rs"]
mod fs_request;

pub(crate) use self::batch_wire::{
    discardable_stage, plan_batches, validate_get, validate_put, BatchPart, FsBatchGet,
    FsBatchOutcome, FsBatchPut, FsBatchStatus, FsTransferCapabilities, BATCH_MAX_BYTES,
    BATCH_MAX_FILES, NONCE_HEX_LEN, TRANSFER_V1_CAPABILITY,
};
pub(crate) use self::fs_request::{
    FsDuplicateSearch, FsHashWalk, FsListBatch, FsRecycle, FsRequest, FsStageFinish,
    FsStorageAnalysis, FsSyncFilesystem, FsWatch,
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
    /// Additive (RV1): neither file, folder nor link (FIFO, socket, device);
    /// never opened. Older hosts omit it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) special: bool,
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
    /// Additive (RV1): the host's further features, the same for every path.
    #[serde(default, skip_serializing_if = "FsHostFeatures::is_absent")]
    pub(crate) features: FsHostFeatures,
    /// Additive (RV1): access of the export that holds the path; absent for
    /// `/`, `/Verbindungen` and from older hosts. Enforced only by hosts
    /// with `export_access_v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) access: Option<ExportAccess>,
}

impl From<crate::vfs::StagedWriteCapabilities> for FsWriteCapabilities {
    fn from(value: crate::vfs::StagedWriteCapabilities) -> Self {
        Self {
            create: value.create,
            replace: value.replace,
            namespace_replace: value.namespace_replace,
            transfer: FsTransferCapabilities::default(),
            features: FsHostFeatures::default(),
            access: None,
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

/// Host features of RV1 beyond transfer v1. Every flag is the capability of
/// its name; older hosts send none, and a client sends the matching request
/// only to a host that offers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsHostFeatures {
    /// `DuplicateSearch`: the host's own duplicate finder.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) duplicate_search_v1: bool,
    /// `HashWalk`: content hashes computed on the host.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) hash_walk_v1: bool,
    /// `ListDirBatch`: folder listings in portions.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) list_batches_v1: bool,
    /// `Recycle`: the host's trash, after a check of the file.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) remote_trash_v1: bool,
    /// `FinishStage`/`SyncFilesystem`: source time, permissions and
    /// durability of finished stages.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) stage_finish_v1: bool,
    /// `StorageAnalysis { compress }`: the tree as one deflate stream.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) analysis_deflate_v1: bool,
    /// `request_id` of `StorageAnalysis`/`DuplicateSearch`: kept for 10
    /// minutes after a lost connection, attachable again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) analysis_reattach_v1: bool,
    /// `WatchExport`: change generations of an export.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) watch_v1: bool,
    /// The host enforces `FsWriteCapabilities::access` and the peer's write
    /// right.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) export_access_v1: bool,
}

impl FsHostFeatures {
    /// What this build offers as a host; each flag is switched on together
    /// with its implementation.
    pub(crate) fn host() -> Self {
        Self::default()
    }

    pub(crate) fn is_absent(&self) -> bool {
        *self == Self::default()
    }

    /// The offered features by capability name, for logs.
    pub(crate) fn names(&self) -> Vec<&'static str> {
        [
            (self.duplicate_search_v1, "duplicate_search_v1"),
            (self.hash_walk_v1, "hash_walk_v1"),
            (self.list_batches_v1, "list_batches_v1"),
            (self.remote_trash_v1, "remote_trash_v1"),
            (self.stage_finish_v1, "stage_finish_v1"),
            (self.analysis_deflate_v1, "analysis_deflate_v1"),
            (self.analysis_reattach_v1, "analysis_reattach_v1"),
            (self.watch_v1, "watch_v1"),
            (self.export_access_v1, "export_access_v1"),
        ]
        .into_iter()
        .filter_map(|(offered, name)| offered.then_some(name))
        .collect()
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
    /// RV1: the host's storage is full; every further write fails the same
    /// way. Older peers read the new kinds as `Unknown`.
    StorageFull,
    /// RV1: the host's user is over its storage quota.
    QuotaExceeded,
    /// RV1: the export, its storage or the peer's right is read-only.
    ReadOnly,
    /// RV1: the file is larger than the host's file system takes.
    FileTooLarge,
    /// RV1: the host's file system cannot take this name.
    InvalidName,
    #[serde(other)]
    Unknown,
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
