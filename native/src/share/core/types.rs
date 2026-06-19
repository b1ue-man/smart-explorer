use crossbeam_channel::{Receiver, Sender};

#[derive(Clone, Debug)]
pub struct RemoteDevice {
    pub device: String,
    pub fingerprint: String,
    pub candidates: Vec<String>,
}

/// What the UI tells the share worker to do.
pub enum ShareCmd {
    /// Announce ourselves for a 1:1 pairing under `code`.
    Pair(String),
    /// Join (or create) a room under `code`.
    JoinRoom(String),
    /// Leave the current pairing/room.
    Leave,
    /// Send these local paths to every current peer (pair = the one peer).
    Send(Vec<String>),
    /// Answer an incoming offer (accept or reject).
    Answer { id: u64, accept: bool },
}

/// What the worker reports back to the UI (drained each frame).
pub enum ShareEvent {
    Status(String),
    Error(String),
    /// Current peers in the session (pair has 0 or 1; room has N).
    Roster(Vec<RemoteDevice>),
    /// An inbound transfer is awaiting accept/reject.
    Incoming {
        id: u64,
        from: String,
        files: Vec<(String, u64)>,
    },
    Progress {
        done: u64,
        total: u64,
    },
    Received {
        count: usize,
        dir: String,
    },
    Sent {
        count: usize,
    },
}

pub(crate) type EventRx = Receiver<ShareEvent>;
pub(crate) type CmdTx = Sender<ShareCmd>;
