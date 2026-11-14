//! Parameters of the RV1 filesystem requests (`FsRequest::StorageAnalysis`
//! to `FsRequest::WatchExport`) and their enums. Every field is additive on
//! the wire; older hosts ignore the fields they do not know.
use serde::{Deserialize, Serialize};

use super::is_false;

/// Content hash of `HashWalk` entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsHashAlgo {
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
pub enum FsStageDurability {
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
pub struct FsStorageAnalysis {
    pub path: String,
    /// `analysis_reattach_v1`: key under which the host keeps this analysis
    /// for 10 minutes, so the client can attach again after a lost
    /// connection. Clients send 32 lowercase hex digits; hosts accept 16 to
    /// 64 characters of `A-Za-z0-9_-` (`analysis_request_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Retained tree nodes the receiver can hold; the host folds further
    /// detail into aggregates (never above its own budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_budget: Option<u64>,
    /// `analysis_deflate_v1`: send the tree data as one deflate stream.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compress: bool,
}

/// `duplicate_search_v1`: the host's own duplicate finder below `path`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDuplicateSearch {
    pub path: String,
    /// Smallest file size that counts (0 counts as 1).
    pub min_bytes: u64,
    /// As `FsStorageAnalysis::request_id` (`analysis_reattach_v1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// `hash_walk_v1`: every folder and regular file below `path` with size and
/// time, the content hash (`algo`) of every file of at least `min_bytes`,
/// and every omission (links, special, unreadable, vanished entries), each
/// folder before its entries; later fields go only to hosts offering them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsHashWalk {
    pub path: String,
    /// `None`: size and time only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algo: Option<FsHashAlgo>,
    #[serde(default)]
    pub min_bytes: u64,
    /// `hash_file_v1`: `path` is one regular file, answered as one entry with
    /// its SHA-256, or none when the host could hash it only over a network.
    #[serde(default, skip_serializing_if = "is_false")]
    pub single_file: bool,
    /// `hash_walk_meta_v1`: every entry carries its listing attributes, and
    /// links and special files come as entries instead of omissions.
    #[serde(default, skip_serializing_if = "is_false")]
    pub meta: bool,
    /// `hash_walk_filter_v1`: fresh digests, the job's filter (with `meta`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<crate::share::fs_response::FsWalkScope>,
    /// `hash_walk_keys_v1` (with `meta`): folder keys (`FsHashWalkMessage::Key`);
    /// `known` = base64 of concatenated 16-byte key prefixes the client holds.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keys: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub known: String,
    /// With `keys`: only every folder's key, or only the runs of `folders`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keys_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
}

/// `list_batches_v1`: the folder `path` in name order and in bounded
/// portions, after the name `cursor` when a broken listing is resumed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsListBatch {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `remote_trash_v1`: move the file `path` into the host's trash, but only
/// while it still has `expected_size` bytes and, when given, this content
/// (`vfs::RecycleExpectation`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsRecycle {
    pub path: String,
    pub expected_size: u64,
    /// Lowercase hex SHA-256 of the whole content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_sha256: Option<String>,
}

/// `stage_finish_v1`: source time, permissions and durability of the
/// client's complete, unpublished stage `staged` (`vfs::StageFinish`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsStageFinish {
    pub staged: String,
    /// Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<i64>,
    /// Unix permission bits; hosts without Unix modes ignore them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    #[serde(default)]
    pub durability: FsStageDurability,
}

/// `stage_finish_v1`: make every stage finished with `Deferred` and
/// published below `path` durable (`vfs::BackendExtensions::sync_filesystem`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsSyncFilesystem {
    pub path: String,
}

/// `watch_v1`: changes below the export subtree `path` until the client
/// closes the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsWatch {
    pub path: String,
}
