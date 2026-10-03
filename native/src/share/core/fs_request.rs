//! Filesystem requests of the Share protocol (`Ctrl::Fs`) and whether each
//! one reads or writes the host's files. Requests of RV1 carry their own
//! parameter struct; on the wire they look like the older struct variants
//! (`{"op": "…", field: …}`), so every field stays additive.
use serde::{Deserialize, Serialize};

use super::{is_false, FsBatchGet, FsBatchPut};

/// What a request does to the host's files. Every request is named in
/// `FsRequest::effect` (no catch-all arm), so a new request cannot reach a
/// read-only export unclassified (FC1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FsEffect {
    /// Reads, describes or watches files, or manages the session
    /// (capabilities, leases, batch status).
    Read,
    /// Creates, changes, moves, recycles or removes files.
    Write,
}

/// Content hash of `HashWalk` entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsHashAlgo {
    /// The sync engine's content signature.
    Md5,
    /// The duplicate finder's content hash.
    Sha256,
    /// An algorithm of a later client; the host answers `Unsupported`.
    #[serde(other)]
    Unknown,
}

/// How durable a finished stage must be before it is published
/// (`vfs::StageDurability`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FsStageDurability {
    #[default]
    NotRequired,
    /// Durable once a later `SyncFilesystem` above it answered `durable`.
    Deferred,
    /// Durable when `FinishStage` answers.
    Now,
    /// A level of a later client; the host answers `Unsupported`.
    #[serde(other)]
    Unknown,
}

/// `StorageAnalysis`: the host's own analysis worker (v2). The RV1 fields are
/// additive; older hosts ignore them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsStorageAnalysis {
    pub(crate) path: String,
    /// `analysis_reattach_v1`: key (16 to 64 lowercase hex digits) under
    /// which the host keeps this analysis for 10 minutes, so the client can
    /// attach again after a lost connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
    /// Retained tree nodes the receiver can hold; the host folds further
    /// detail into aggregates (never above its own budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node_budget: Option<u64>,
    /// `analysis_deflate_v1`: send the tree data as one deflate stream.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) compress: bool,
}

/// `duplicate_search_v1`: the host's own duplicate finder below `path`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsDuplicateSearch {
    pub(crate) path: String,
    /// Smallest file size that counts (0 counts as 1).
    pub(crate) min_bytes: u64,
    /// As `FsStorageAnalysis::request_id` (`analysis_reattach_v1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
}

/// `hash_walk_v1`: every folder and regular file below `path` with size and
/// time, the content hash (`algo`) of every file of at least `min_bytes`,
/// and every omission (links, special, unreadable, vanished entries).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsHashWalk {
    pub(crate) path: String,
    /// `None`: size and time only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) algo: Option<FsHashAlgo>,
    #[serde(default)]
    pub(crate) min_bytes: u64,
}

/// `list_batches_v1`: the folder `path` in name order and in bounded
/// portions, after the name `cursor` when a broken listing is resumed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsListBatch {
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
}

/// `remote_trash_v1`: move the file `path` into the host's trash, but only
/// while it still has `expected_size` bytes and, when given, this content
/// (`vfs::RecycleExpectation`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsRecycle {
    pub(crate) path: String,
    pub(crate) expected_size: u64,
    /// Lowercase hex SHA-256 of the whole content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expected_sha256: Option<String>,
}

/// `stage_finish_v1`: source time, permissions and durability of the
/// client's complete, unpublished stage `staged` (`vfs::StageFinish`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsStageFinish {
    pub(crate) staged: String,
    /// Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mtime_ms: Option<i64>,
    /// Unix permission bits; hosts without Unix modes ignore them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mode: Option<u32>,
    #[serde(default)]
    pub(crate) durability: FsStageDurability,
}

/// `stage_finish_v1`: make every stage finished with `Deferred` and
/// published below `path` durable (`vfs::BackendExtensions::sync_filesystem`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsSyncFilesystem {
    pub(crate) path: String,
}

/// `watch_v1`: changes below the export subtree `path` until the client
/// closes the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsWatch {
    pub(crate) path: String,
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
    /// `literal_children_v1`: provider-encoded child of this unchanged parent.
    SyncChildPath { parent: String, literal_name: String },
    WalkTree {
        path: String,
    },
    StorageSnapshot {
        path: String,
    },
    StorageAnalysis(FsStorageAnalysis),
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
    /// RV1, `duplicate_search_v1`.
    DuplicateSearch(FsDuplicateSearch),
    /// RV1, `hash_walk_v1`.
    HashWalk(FsHashWalk),
    /// RV1, `list_batches_v1`.
    ListDirBatch(FsListBatch),
    /// RV1, `remote_trash_v1`.
    Recycle(FsRecycle),
    /// RV1, `stage_finish_v1`.
    FinishStage(FsStageFinish),
    /// RV1, `stage_finish_v1`.
    SyncFilesystem(FsSyncFilesystem),
    /// RV1, `watch_v1`.
    WatchExport(FsWatch),
}

impl FsRequest {
    fn effect(&self) -> FsEffect {
        match self {
            Self::Capabilities { .. }
            | Self::ReleaseLease
            | Self::ListDir { .. }
            | Self::ListDirBatch(_)
            | Self::Stat { .. }
            | Self::SyncChildPath { .. }
            | Self::WalkTree { .. }
            | Self::StorageSnapshot { .. }
            | Self::StorageAnalysis(_)
            | Self::DuplicateSearch(_)
            | Self::HashWalk(_)
            | Self::WatchExport(_)
            | Self::Read { .. }
            | Self::ReadAt { .. }
            | Self::GetBatch { .. }
            | Self::PutBatchStatus { .. } => FsEffect::Read,
            Self::Write { .. }
            | Self::WriteNew { .. }
            | Self::WriteDone
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
            | Self::Recycle(_)
            | Self::FinishStage(_)
            | Self::SyncFilesystem(_) => FsEffect::Write,
        }
    }

    /// Whether the request writes: refused on read-only exports and for
    /// peers without write right (FC1), re-admitted through the mount lease.
    pub(in crate::share) fn mutates_filesystem(&self) -> bool {
        self.effect() == FsEffect::Write
    }

    /// Batches admit every entry again, exactly like one single request.
    pub(in crate::share) fn is_batch(&self) -> bool {
        matches!(self, Self::PutBatch { .. } | Self::GetBatch { .. })
    }

    /// Requests a host before transfer v1 cannot parse; clients send them
    /// only after that host's Capabilities advertised v1.
    pub(in crate::share) fn is_transfer_v1(&self) -> bool {
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
