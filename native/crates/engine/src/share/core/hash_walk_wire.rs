//! Messages of a host-side hash walk (`hash_walk_v1`, `FsResponse::HashWalk`;
//! listing attributes with `hash_walk_meta_v1`, folder keys with
//! `hash_walk_keys_v1`, fresh digests and the job's filter with
//! `hash_walk_filter_v1`).
use serde::{Deserialize, Serialize};

use super::list_batch_wire::FsOmission;
use crate::agent_proto::WalkFilterSpec;
use crate::vfs::{HashWalkEntry, HashWalkItem, HashWalkMeta, KeyTally};

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
pub enum FsHashWalkMessage {
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
    /// `hash_walk_keys_v1`: the run of the folder `rel` begins (every item
    /// up to the next `Run` or `Key` is one of it).
    Run {
        rel: String,
    },
    /// `hash_walk_keys_v1`: the key of the folder `rel` (64 lowercase hex
    /// digits); `same`: its subtree is the known one and was not sent;
    /// `tally`: what its subtree holds (a keys-only walk sends it).
    Key {
        rel: String,
        key: String,
        #[serde(default, skip_serializing_if = "is_false")]
        same: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tally: Option<KeyTally>,
    },
}

/// One entry below the walk root (`vfs::HashWalkEntry`): a folder, a regular
/// file and, with `FsHashWalk::meta`, a link or special file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsHashEntry {
    pub rel: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_dir: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime_ms: i64,
    /// Lowercase hex digest of the requested algorithm (regular files only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// `hash_walk_meta_v1`: present exactly when the request asked for `meta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<FsHashMeta>,
}

/// The listing attributes of one entry (`vfs::HashWalkMeta`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsHashMeta {
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub system: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub btime_ms: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_symlink: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub special: bool,
}

impl FsHashEntry {
    /// JSON bytes this entry adds to a portion (estimate).
    pub fn wire_bytes(&self) -> usize {
        self.rel.len()
            + self.digest.as_ref().map_or(0, String::len)
            + if self.meta.is_some() { 160 } else { 96 }
    }

    /// A regular file (neither folder, link nor special file).
    pub fn is_regular(&self) -> bool {
        !self.is_dir
            && self
                .meta
                .is_none_or(|meta| !meta.is_symlink && !meta.special)
    }
}

/// `hash_walk_filter_v1` (`FsHashWalk::scope`): digests read from the files
/// now, and the sync job's filter (`agent_proto::walk_filter`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsWalkScope {
    #[serde(default, skip_serializing_if = "is_false")]
    pub fresh: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<FsWalkFilter>,
}

/// The filter of a walk (`agent_proto::WalkFilterSpec`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsWalkFilter {
    #[serde(default, skip_serializing_if = "is_false")]
    pub skip_hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub backslash_separates: bool,
    #[serde(default)]
    pub min_size: u64,
    #[serde(default)]
    pub max_size: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub after_mtime_ms: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub before_mtime_ms: i64,
}

impl FsWalkFilter {
    /// `spec` with its patterns, in order, as far as they fit the text a
    /// request carries beside its largest known-key text (about 214 KiB) or
    /// folder list (160 KiB) below the request frame limit
    /// (`framing::MAX_REQUEST_CTRL_FRAME`, 256 KiB): `WalkFilterSpec::bounded`,
    /// the bound `agent_proto::host_patterns` already keeps every job within,
    /// so the agent, this host and the local side apply the same list.
    pub fn bounded(spec: &WalkFilterSpec) -> Self {
        let spec = spec.bounded();
        Self {
            skip_hidden: spec.skip_hidden,
            patterns: spec.patterns,
            backslash_separates: spec.backslash_separates,
            min_size: spec.min_size,
            max_size: spec.max_size,
            after_mtime_ms: spec.after_mtime_ms,
            before_mtime_ms: spec.before_mtime_ms,
        }
    }
}

impl From<FsWalkFilter> for WalkFilterSpec {
    fn from(filter: FsWalkFilter) -> Self {
        Self {
            skip_hidden: filter.skip_hidden,
            patterns: filter.patterns,
            backslash_separates: filter.backslash_separates,
            min_size: filter.min_size,
            max_size: filter.max_size,
            after_mtime_ms: filter.after_mtime_ms,
            before_mtime_ms: filter.before_mtime_ms,
        }
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
