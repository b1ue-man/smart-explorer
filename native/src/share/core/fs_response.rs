use super::wire::{FsBatchStatus, FsErrorKind, FsMeta, FsWalkNode, FsWriteCapabilities};
use serde::{Deserialize, Serialize};

#[path = "duplicate_wire.rs"]
mod duplicate_wire;
#[path = "hash_walk_wire.rs"]
mod hash_walk_wire;
#[path = "list_batch_wire.rs"]
mod list_batch_wire;
#[path = "watch_wire.rs"]
mod watch_wire;

pub(crate) use self::duplicate_wire::{
    FsDuplicateFile, FsDuplicateGroup, FsDuplicateMessage, FsDuplicateProgress, FsDuplicateSummary,
    GROUP_PORTION_BYTES,
};
pub(crate) use self::hash_walk_wire::{FsHashEntry, FsHashWalkMessage};
pub(crate) use self::list_batch_wire::{
    FsOmission, FsOmissionReason, LIST_BATCH_MAX_BYTES, LIST_BATCH_MAX_ENTRIES, META_JSON_BYTES,
    STREAM_FRAME_LIMIT,
};
pub(crate) use self::watch_wire::{FsWatchEvent, ALIVE_SECS, MAX_NOTICE_PATHS, SILENT_SECS};

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
        #[serde(default)]
        storage_analysis_v2: bool,
    },
    Entries {
        entries: Vec<FsMeta>,
    },
    Meta {
        meta: FsMeta,
    },
    /// `literal_children_v1`: still in the same virtual parent/root/lease.
    ChildPath { path: String },
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
    Analysis {
        message: crate::analytics::analysis_transfer::AnalysisMessage,
    },
    Data {
        size: u64,
    },
    Ready,
    Ok,
    /// Transfer v1: reply to a committed PutBatch and to PutBatchStatus.
    Batch {
        status: FsBatchStatus,
    },
    /// RV1 `Recycle`: `moved` false = the file no longer matched the
    /// expectation and stayed where it is.
    Recycle {
        moved: bool,
    },
    /// RV1 `FinishStage` (`vfs::StageFinished`).
    StageFinished {
        mtime_applied: bool,
        durable: bool,
    },
    /// RV1 `SyncFilesystem`: whether the finished stages below the path are
    /// durable now.
    Synced {
        durable: bool,
    },
    /// RV1 `ListDirBatch`: one portion of the folder, in name order.
    EntriesBatch {
        entries: Vec<FsMeta>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        omitted: Vec<FsOmission>,
    },
    /// RV1 `ListDirBatch`: the listing is complete; totals for a check.
    EntriesDone {
        entries: u64,
        omitted: u64,
    },
    /// RV1 `DuplicateSearch`.
    Duplicates {
        message: FsDuplicateMessage,
    },
    /// RV1 `HashWalk`.
    HashWalk {
        message: FsHashWalkMessage,
    },
    /// RV1 `WatchExport`.
    Watch {
        event: FsWatchEvent,
    },
    Err {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<FsErrorKind>,
        msg: String,
    },
}
