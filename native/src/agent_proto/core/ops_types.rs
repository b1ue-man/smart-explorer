//! Payloads of the extension frames (`ext-v1`): the optional backend
//! operations a client reaches through an agent or the background service
//! (tolerant listings, hash walks with omissions, the duplicate search of the
//! storing host, recycling, stage finishing, target limits, change
//! subscriptions). Codes stay plain integers so this module keeps no
//! dependency beyond the standard library (the standalone agent includes it).

/// Reply text of an extension the serving side does not offer; the client
/// reads it as "unsupported" (`Ok(None)`/`Ok(false)`), never as a failure.
pub const UNSUPPORTED_EXTENSION: &str = "SE_EXTENSION_UNSUPPORTED_V1";

/// `Query` kinds; every answer is an `Answer(u64)`.
pub mod query {
    /// 1 = the storing host searches duplicates below the path.
    pub const DUPLICATE_SEARCH: u8 = 1;
    /// 1 = the walk with digests runs next to the data (no download).
    pub const HASH_WALK: u8 = 2;
    /// 1 = the path can go to its device's trash.
    pub const RECYCLE: u8 = 3;
    /// 0 = none, 1 = pushed notices, 2 = polled cursor.
    pub const CHANGE_SIGNAL: u8 = 4;
    /// Flush what was published below the path; 1 = done durably.
    pub const SYNC_FILESYSTEM: u8 = 5;
}

/// `WireOmission::reason` codes.
pub mod omission {
    pub const LINK: u8 = 0;
    pub const SPECIAL: u8 = 1;
    pub const UNREADABLE: u8 = 2;
    pub const VANISHED: u8 = 3;
    pub const UNREPRESENTABLE: u8 = 4;
}

/// Digest algorithm codes of `WalkHashed2` and duplicate groups.
pub mod digest {
    pub const NONE: u8 = 0;
    pub const MD5: u8 = 1;
    pub const SHA256: u8 = 2;
}

/// One existing entry a listing or walk had to leave out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireOmission {
    /// Entry name (listing) or path relative to the walk root (walk).
    pub rel: String,
    pub reason: u8,
    pub detail: String,
}

/// Live state of a duplicate search on the storing host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireReclaimProgress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub fingerprinted: u64,
    pub hashed: u64,
    pub candidates: u64,
    /// 0 walking, 1 comparing ends, 2 comparing contents, 3 grouping.
    pub phase: u8,
    pub files_total: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub current: String,
}

/// One copy of a duplicate group (a path of the serving backend).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireDuplicateItem {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub backend_id: Option<String>,
}

/// Files with equal content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireDuplicateGroup {
    pub size: u64,
    /// `digest::MD5` or `digest::SHA256`.
    pub algorithm: u8,
    pub hex: String,
    /// 0 content SHA-256, 1 provider MD5, 2 agent MD5.
    pub evidence: u8,
    pub reclaimable: u64,
    pub items: Vec<WireDuplicateItem>,
}

/// Everything a duplicate search reports besides its groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireDuplicateSummary {
    pub files: u64,
    pub bytes: u64,
    pub candidates: u64,
    pub compared: u64,
    pub groups: u64,
    /// Protected areas (area, omitted entries).
    pub protected: Vec<(String, u64)>,
    pub errors: Vec<String>,
    pub suppressed_errors: u64,
    pub limits: Vec<String>,
    pub root_error: Option<String>,
}

/// What a target below a root can store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireTargetLimits {
    pub windows_names: bool,
    /// 0 unknown, 1 UTF-8 bytes, 2 UTF-16 units.
    pub name_limit: u8,
    pub name_max: u64,
    pub max_file_size: Option<u64>,
    /// 0 nanos, 1 millis, 2 ten millis, 3 seconds, 4 two seconds,
    /// 5 minutes, 6 days, 7 unknown.
    pub precision: u8,
}

/// One notice of a change subscription.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireChange {
    /// 0 ready (complete), 1 changed, 2 overflow, 3 ended, 4 ready (partial).
    pub kind: u8,
    pub generation: Option<u64>,
    pub paths: Vec<String>,
    /// Why the subscription ended (`kind` 3).
    pub text: String,
}
