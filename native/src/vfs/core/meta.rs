//! Plain data types of the storage interface (entries, changes, search and
//! walk hits). The `Backend` trait itself lives in `core.rs`.
use std::io;

/// Backend-neutral directory entry / file metadata. Fields a remote backend
/// can't supply (`btime`, `hidden`, `system`) default to `0` / `false`.
#[derive(Clone, Debug, Default)]
pub struct VfsMeta {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub hidden: bool,
    pub system: bool,
    /// Backend-unique id when names alone aren't unique (e.g. Google Drive
    /// keys by file-id and allows duplicate names in one folder). None = the
    /// path/name uniquely identifies the item (local, SFTP, FTP, WebDAV).
    pub id: Option<String>,
    /// Server-provided content MD5 (hex), if the backend exposes one for free in
    /// its listing - Google Drive `md5Checksum`, Nextcloud/ownCloud
    /// `oc:checksums`. Lets checksum-mode compare without downloading the file.
    /// None = not provided (local/SFTP/FTP, Google-Docs/folders).
    pub content_md5: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Upsert,
    Remove,
}

/// One backend-reported change. `rel` is optional because ID-addressed remotes
/// such as Drive may report a stable file id and parent id; the sync index can
/// resolve that into a relative path from previous state.
#[derive(Clone, Debug)]
pub struct VfsChange {
    pub kind: ChangeKind,
    pub rel: Option<String>,
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: Option<String>,
    pub meta: Option<VfsMeta>,
}

#[derive(Clone, Debug, Default)]
pub struct VfsChangeBatch {
    pub changes: Vec<VfsChange>,
    pub new_cursor: Option<String>,
    pub reset: bool,
}

pub type VfsResult<T> = io::Result<T>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteDisposition {
    Recycle,
    Permanent,
    Unsupported,
}

/// One exact, read-only-planned duplicate cleanup target. Backends with stable
/// IDs must populate `id` so applying an earlier safety preflight cannot delete
/// a different same-name object after concurrent changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DedupeCandidate {
    pub path: String,
    pub id: Option<String>,
}

/// One server-side search match (path relative to the search root).
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
}

/// One entry of a server-side signature walk (path relative to the walk root).
/// `md5` is the hex content hash, present only for files when hashing was asked.
#[derive(Clone, Debug)]
pub struct HashHit {
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub md5: Option<String>,
}
