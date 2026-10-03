//! Messages of a host-side hash walk (`hash_walk_v1`, `FsResponse::HashWalk`).
use serde::{Deserialize, Serialize};

use super::list_batch_wire::FsOmission;
use crate::vfs::{HashWalkEntry, HashWalkItem};

fn is_false(value: &bool) -> bool {
    !*value
}

/// The stream of one hash walk: portions, heartbeats while a large file is
/// hashed, and the closing totals the client checks its count against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "m", rename_all = "snake_case")]
pub(crate) enum FsHashWalkMessage {
    Progress {
        files: u64,
        bytes: u64,
    },
    Batch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entries: Vec<FsHashEntry>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        omitted: Vec<FsOmission>,
    },
    Done {
        entries: u64,
        omitted: u64,
    },
}

/// One folder or regular file below the walk root (`vfs::HashWalkEntry`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsHashEntry {
    pub(crate) rel: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) is_dir: bool,
    #[serde(default)]
    pub(crate) size: u64,
    #[serde(default)]
    pub(crate) mtime_ms: i64,
    /// Lowercase hex digest of the requested algorithm (files only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) digest: Option<String>,
}

impl FsHashEntry {
    /// JSON bytes this entry adds to a portion (estimate).
    pub(crate) fn wire_bytes(&self) -> usize {
        self.rel.len() + self.digest.as_ref().map_or(0, String::len) + 96
    }
}

impl From<HashWalkEntry> for FsHashEntry {
    fn from(entry: HashWalkEntry) -> Self {
        Self {
            rel: entry.rel,
            is_dir: entry.is_dir,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            digest: entry.digest,
        }
    }
}

impl From<FsHashEntry> for HashWalkItem {
    fn from(entry: FsHashEntry) -> Self {
        HashWalkItem::Entry(HashWalkEntry {
            rel: entry.rel,
            is_dir: entry.is_dir,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            digest: entry.digest,
        })
    }
}
