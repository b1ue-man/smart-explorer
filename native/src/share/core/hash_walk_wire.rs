//! Messages of a host-side hash walk (`hash_walk_v1`, `FsResponse::HashWalk`;
//! listing attributes with `hash_walk_meta_v1`).
use serde::{Deserialize, Serialize};

use super::list_batch_wire::FsOmission;
use crate::vfs::{HashWalkEntry, HashWalkItem, HashWalkMeta};

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &i64) -> bool {
    *value == 0
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

/// One entry below the walk root (`vfs::HashWalkEntry`): a folder, a regular
/// file and, with `FsHashWalk::meta`, a link or special file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsHashEntry {
    pub(crate) rel: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) is_dir: bool,
    #[serde(default)]
    pub(crate) size: u64,
    #[serde(default)]
    pub(crate) mtime_ms: i64,
    /// Lowercase hex digest of the requested algorithm (regular files only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) digest: Option<String>,
    /// `hash_walk_meta_v1`: present exactly when the request asked for `meta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) meta: Option<FsHashMeta>,
}

/// The listing attributes of one entry (`vfs::HashWalkMeta`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsHashMeta {
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) system: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) btime_ms: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) is_symlink: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) special: bool,
}

impl FsHashEntry {
    /// JSON bytes this entry adds to a portion (estimate).
    pub(crate) fn wire_bytes(&self) -> usize {
        self.rel.len()
            + self.digest.as_ref().map_or(0, String::len)
            + if self.meta.is_some() { 160 } else { 96 }
    }

    /// A regular file (neither folder, link nor special file).
    pub(crate) fn is_regular(&self) -> bool {
        !self.is_dir
            && self
                .meta
                .is_none_or(|meta| !meta.is_symlink && !meta.special)
    }
}

impl From<HashWalkMeta> for FsHashMeta {
    fn from(meta: HashWalkMeta) -> Self {
        Self {
            hidden: meta.hidden,
            system: meta.system,
            btime_ms: meta.btime_ms,
            is_symlink: meta.is_symlink,
            special: meta.special,
        }
    }
}

impl From<FsHashMeta> for HashWalkMeta {
    fn from(meta: FsHashMeta) -> Self {
        Self {
            hidden: meta.hidden,
            system: meta.system,
            btime_ms: meta.btime_ms,
            is_symlink: meta.is_symlink,
            special: meta.special,
        }
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
            meta: entry.meta.map(Into::into),
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
            meta: entry.meta.map(Into::into),
        })
    }
}
