use serde::{Deserialize, Serialize};

use super::types::PeerPresence;

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
    HelloOk,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum FsRequest {
    ListDir { path: String },
    Stat { path: String },
    Read { path: String },
    Write { path: String },
    WriteDone,
    MkdirAll { path: String },
    Rename { src: String, dst: String },
    CopyFile { src: String, dst: String },
    RemoveFile { path: String },
    RemoveDir { path: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "r", rename_all = "snake_case")]
pub(crate) enum FsResponse {
    Entries { entries: Vec<FsMeta> },
    Meta { meta: FsMeta },
    Data { size: u64 },
    Ready,
    Ok,
    Err { msg: String },
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
}
