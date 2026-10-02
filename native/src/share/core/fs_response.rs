use super::wire::{FsBatchStatus, FsErrorKind, FsMeta, FsWalkNode, FsWriteCapabilities};
use serde::{Deserialize, Serialize};

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
    Err {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<FsErrorKind>,
        msg: String,
    },
}
