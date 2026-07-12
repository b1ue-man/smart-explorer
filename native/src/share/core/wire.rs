use serde::{Deserialize, Serialize};

use super::direct_protocol::{
    DirectRequestId, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::types::{ExecRequest, ExecResult, PeerPresence};

pub(crate) const TRACKED_DIRECT_CAPABILITY: &str = "tracked_direct_v1";

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
    Request { request: SignedDirectRequest },
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

/// A deliberately small, additive classification for filesystem failures.
/// The message remains the source of detail; this kind exists only where a
/// caller must make a safe control-flow decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsErrorKind {
    NotFound,
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum FsRequest {
    ListDir { path: String },
    Stat { path: String },
    WalkTree { path: String },
    Read { path: String },
    Write { path: String },
    WriteDone,
    MkdirAll { path: String },
    Rename { src: String, dst: String },
    RenameNoReplace { src: String, dst: String },
    PromoteStaged { staged: String, destination: String },
    CopyFile { src: String, dst: String },
    RemoveFile { path: String },
    RemoveDir { path: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "r", rename_all = "snake_case")]
pub(crate) enum FsResponse {
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
    PeerHello { hello: PeerHello },
    PeerHelloOk,
    Ping { nonce: String },
    Pong { nonce: String },
    Fs { req: FsRequest },
    FsResp { resp: FsResponse },
    Exec { req: ExecRequest },
    ExecResp { result: ExecResult },
    ExecErr { msg: String },
}
