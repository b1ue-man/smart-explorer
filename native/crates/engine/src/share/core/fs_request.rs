//! Filesystem requests of the Share protocol (`Ctrl::Fs`) and whether each
//! one reads or writes the host's files. Requests of RV1 carry their own
//! parameter struct; on the wire they look like the older struct variants
//! (`{"op": "…", field: …}`), so every field stays additive.
use serde::{Deserialize, Serialize};

use super::{
    is_false, FsBatchGet, FsBatchPut, FsCreateDirs, FsSegmentedFinish, FsSegmentedOpen,
    FsStageWrite,
};

/// What a request does to the host's files. Every request is named in
/// `FsRequest::effect` (no catch-all arm), so a new request cannot reach a
/// read-only export unclassified (FC1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FsEffect {
    /// Reads, describes or watches files, or manages the session
    /// (capabilities, leases, batch status, login target).
    Read,
    /// Creates, changes, moves, recycles or removes files.
    Write,
}

#[path = "fs_request_ext.rs"]
mod ext;
pub use self::ext::{
    FsDuplicateSearch, FsHashAlgo, FsHashWalk, FsListBatch, FsRecycle, FsStageDurability,
    FsStageFinish, FsStorageAnalysis, FsSyncFilesystem, FsWatch,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FsRequest {
    /// `folder_reservation_v1`: strict atomic reservation, never legacy fallback.
    CreateDirExclusive {
        path: String,
    },
    /// `folder_reservation_v1`: stable identity of the actual plain directory.
    DirectoryIdentity {
        path: String,
    },
    /// `file_identity_v1`: backend-attested regular-file object identity.
    FileIdentity {
        path: String,
    },
    /// Technical primitive support; permission and identity remain separate.
    DirectoryReservationSupported {
        path: String,
    },
    Capacity {
        path: String,
    },
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
    SyncChildPath {
        parent: String,
        literal_name: String,
    },
    WalkTree {
        path: String,
    },
    StorageSnapshot {
        path: String,
    },
    StorageAnalysis(FsStorageAnalysis),
    Read {
        path: String,
        /// `lz4_v1`: the host may send content blocks (`TAG_DATA_LZ4`).
        #[serde(default, skip_serializing_if = "is_false")]
        lz4: bool,
    },
    Write {
        path: String,
        /// `lz4_v1`: the client may send content blocks.
        #[serde(default, skip_serializing_if = "is_false")]
        lz4: bool,
    },
    WriteNew {
        path: String,
        /// `lz4_v1`: the client may send content blocks.
        #[serde(default, skip_serializing_if = "is_false")]
        lz4: bool,
        /// `evidence_v1`: the commit answers `Written` with the stage's
        /// evidence and whether `probe` (a sibling) existed.
        #[serde(default, skip_serializing_if = "is_false")]
        evidence: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        probe: Option<String>,
        /// `stage_mtime_v1`: the source time (Unix ms) the host gives the new
        /// stage at its commit, before it reads the stage's evidence, so no
        /// `FinishStage` of its own follows. Sent only to a host offering it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mtime_ms: Option<i64>,
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
    /// `folder_reservation_v1`: atomic publication of an owned directory.
    RenameReservedDirectory {
        src: String,
        dst: String,
    },
    PromoteStaged {
        staged: String,
        destination: String,
    },
    /// `reversible_replace_v1`: retain the original at a journaled sibling.
    ReplaceStagedReversible(FsReversibleReplace),
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
    /// `remove_tree_v1`: the plain folder `path` and everything below it,
    /// deleted on the host in one stream (`FsResponse::RemoveTree`); links
    /// are removed as entries, never followed. Sent only to hosts that offer
    /// the feature; `RemoveDir` stays for older clients.
    RemoveTree {
        path: String,
    },
    /// Transfer v1: many small new files in one stream. Each entry lands in a
    /// private stage named with `nonce`; nothing is published before the
    /// client's WriteDone, and nothing is ever replaced.
    PutBatch {
        nonce: String,
        entries: Vec<FsBatchPut>,
        /// `evidence_v1`: `BatchReady` names existing destinations before the
        /// commit, and each published outcome carries its evidence.
        #[serde(default, skip_serializing_if = "is_false")]
        evidence: bool,
    },
    /// Transfer v1: the state of this principal's PutBatch `nonce` after its
    /// reply was lost.
    PutBatchStatus {
        nonce: String,
    },
    /// Transfer v1: many small files in one stream, in request order.
    GetBatch {
        items: Vec<FsBatchGet>,
        /// `evidence_v1`: item headers are `EvidencedData`.
        #[serde(default, skip_serializing_if = "is_false")]
        evidence: bool,
    },
    /// Transfer v1: read from `offset` (resume) or by provider ID. With
    /// `ranges_v1` the read may name its length: `len` bytes from `offset`
    /// (0 = to the end), cut at the end of the file.
    ReadAt {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        offset: u64,
        #[serde(default, skip_serializing_if = "is_zero")]
        len: u64,
        /// `lz4_v1`: the host may send content blocks (`TAG_DATA_LZ4`).
        #[serde(default, skip_serializing_if = "is_false")]
        lz4: bool,
        /// `evidence_v1`: the header is `EvidencedData`.
        #[serde(default, skip_serializing_if = "is_false")]
        evidence: bool,
    },
    /// `ranges_v1`: create the exclusive stage of the file `path` (the host
    /// picks its name and holds the handle) and answer
    /// `SegmentedStageReady`.
    SegmentedStageOpen(FsSegmentedOpen),
    /// `ranges_v1`: write one range of an open stage; the client then sends
    /// exactly `len` bytes and ends its stream.
    StageWriteAt(FsStageWrite),
    /// `ranges_v1`: every range of the stage has arrived.
    SegmentedStageFinish(FsSegmentedFinish),
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
        /// `evidence_v1`: answered `Published` with the published file.
        #[serde(default, skip_serializing_if = "is_false")]
        evidence: bool,
    },
    /// `create_dirs_v1`: many folders, parents first, in one request.
    CreateDirs(FsCreateDirs),
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
    /// `login_target_v1` (E43): the public target of the host's machine
    /// login gateway, answered `FsResponse::LoginTarget`. Reads no file.
    LoginTarget,
}

impl FsRequest {
    fn effect(&self) -> FsEffect {
        match self {
            Self::DirectoryReservationSupported { .. }
            | Self::DirectoryIdentity { .. }
            | Self::FileIdentity { .. }
            | Self::Capacity { .. }
            | Self::Capabilities { .. }
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
            | Self::PutBatchStatus { .. }
            | Self::LoginTarget => FsEffect::Read,
            Self::CreateDirExclusive { .. }
            | Self::Write { .. }
            | Self::WriteNew { .. }
            | Self::WriteDone
            | Self::MkdirAll { .. }
            | Self::CreateDir { .. }
            | Self::CreateDirs(_)
            | Self::Rename { .. }
            | Self::RenameNoReplace { .. }
            | Self::RenameReservedDirectory { .. }
            | Self::PromoteStaged { .. }
            | Self::ReplaceStagedReversible(_)
            | Self::PromoteNoReplace { .. }
            | Self::CopyFile { .. }
            | Self::RemoveFile { .. }
            | Self::RemoveDir { .. }
            | Self::RemoveTree { .. }
            | Self::DiscardStage { .. }
            | Self::SegmentedStageOpen(_)
            | Self::StageWriteAt(_)
            | Self::SegmentedStageFinish(_)
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

    /// Whether the request lets the other side pack the content of its
    /// stream (`lz4_v1`).
    pub(in crate::share) fn asks_lz4(&self) -> bool {
        match self {
            Self::Read { lz4, .. }
            | Self::Write { lz4, .. }
            | Self::WriteNew { lz4, .. }
            | Self::ReadAt { lz4, .. } => *lz4,
            Self::StageWriteAt(write) => write.lz4,
            _ => false,
        }
    }

    /// The open stage a ranged-upload request writes into.
    pub(in crate::share) fn range_stage(&self) -> Option<&str> {
        match self {
            Self::StageWriteAt(write) => Some(&write.stage),
            Self::SegmentedStageFinish(finish) => Some(&finish.stage),
            _ => None,
        }
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
                | Self::CreateDirs(_)
                | Self::PromoteNoReplace { .. }
                | Self::DiscardStage { .. }
                | Self::SegmentedStageOpen(_)
                | Self::StageWriteAt(_)
                | Self::SegmentedStageFinish(_)
        )
    }
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[cfg(test)]
#[path = "fs_request_tests.rs"]
mod reservation_compatibility_tests;

/// The caller persists recovery intent before sending this mutation once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsReversibleReplace {
    pub staged: String,
    pub destination: String,
    pub retained: String,
}
