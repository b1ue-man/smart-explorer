use serde::{Deserialize, Serialize};

use super::types::RemoteDevice;

#[derive(Serialize)]
pub(crate) struct Hello {
    pub(crate) t: &'static str,
    pub(crate) mode: String,
    pub(crate) code: String,
    pub(crate) device: String,
    pub(crate) listen_port: u16,
    pub(crate) lan: Vec<String>,
    pub(crate) pubkey: String,
}

#[derive(Deserialize)]
pub(crate) struct SrvMember {
    device: String,
    candidates: Vec<String>,
    pubkey: String,
}

impl From<SrvMember> for RemoteDevice {
    fn from(m: SrvMember) -> Self {
        RemoteDevice {
            device: m.device,
            fingerprint: m.pubkey,
            candidates: m.candidates,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub(crate) enum SrvMsg {
    Peer {
        device: String,
        candidates: Vec<String>,
        pubkey: String,
    },
    Roster {
        members: Vec<SrvMember>,
    },
    Joined {
        member: SrvMember,
    },
    Left {
        #[allow(dead_code)]
        device: String,
        pubkey: String,
    },
    Error {
        msg: String,
    },
}

#[derive(Serialize, Deserialize)]
pub(crate) struct FileMeta {
    pub(crate) name: String,
    pub(crate) size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "c")]
pub(crate) enum Ctrl {
    Offer { from: String, files: Vec<FileMeta> },
    Accept,
    Reject,
    FileStart { name: String, size: u64 },
    FileEnd,
    Done,
}
