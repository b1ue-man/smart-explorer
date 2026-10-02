//! Data types of the optional backend extensions (`BackendExtensions`):
//! tolerant listings, target limits, stage finishing, host-side duplicate
//! search, hash walks, recycling and change subscriptions.
use super::VfsMeta;
use crate::types::{win32_name_issue, Win32NameIssue};

/// Why an existing entry is missing from a listing or walk. An omission is
/// never an absence: its counterpart and baseline entries stay protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OmissionReason {
    /// Symlink, junction or name-surrogate reparse point (walk boundary).
    Link,
    /// FIFO, socket or device: there is no data stream to read.
    Special,
    /// It exists, but its metadata, listing or content could not be read.
    Unreadable,
    /// It disappeared between being listed and being read.
    Vanished,
    /// Its name is no path of this interface (not valid Unicode, a name
    /// Win32 cannot address, …).
    Unrepresentable,
}

/// One existing entry a listing or walk had to leave out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VfsOmission {
    /// Entry name (listing) or path relative to the walk root (walk); lossy
    /// (U+FFFD) where the stored name is not valid Unicode. Everything below
    /// it counts as omitted too.
    pub rel: String,
    pub reason: OmissionReason,
    /// Cause for the run report (OS or protocol error text).
    pub detail: String,
}

/// A directory listing that keeps going past individual entries. `Ok` means
/// every existing child is in `entries` or in `omitted`; a listing that
/// cannot vouch for that (the enumeration itself broke off) is an error.
/// Links and special files stay in `entries` with their flags set.
#[derive(Clone, Debug, Default)]
pub struct VfsListing {
    pub entries: Vec<VfsMeta>,
    pub omitted: Vec<VfsOmission>,
}

impl VfsListing {
    /// A listing without omissions.
    pub fn complete(entries: Vec<VfsMeta>) -> Self {
        Self {
            entries,
            omitted: Vec::new(),
        }
    }
}

/// Resolution of stored modification times below a root; comparisons across
/// two sides use the coarser of both (`coarser`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MtimePrecision {
    /// ext4, btrfs, xfs, f2fs, tmpfs, NTFS/ReFS (100 ns), SMB.
    Nanos,
    /// Google Drive, FTP `MDTM` with fraction.
    Millis,
    /// exFAT (10 ms steps).
    TenMillis,
    /// SFTP, FTP `MDTM`/`MLSD`, WebDAV/HTTP dates.
    Seconds,
    /// FAT/FAT32/VFAT: 2 s steps in local time, so a daylight-saving change
    /// shifts the reported time by exactly one hour (`local_time_shifts`).
    TwoSeconds,
    /// FTP `LIST` rows of recent entries (hour and minute).
    Minutes,
    /// FTP `LIST` rows of older entries (date only).
    Days,
    /// Not determined: only identical times are the same.
    #[default]
    Unknown,
}

impl MtimePrecision {
    /// One step in milliseconds; `None` for `Unknown`.
    pub const fn step_ms(self) -> Option<u64> {
        match self {
            Self::Nanos | Self::Millis => Some(1),
            Self::TenMillis => Some(10),
            Self::Seconds => Some(1_000),
            Self::TwoSeconds => Some(2_000),
            Self::Minutes => Some(60_000),
            Self::Days => Some(86_400_000),
            Self::Unknown => None,
        }
    }

    /// The precision a comparison between two sides can rely on.
    pub fn coarser(self, other: Self) -> Self {
        self.max(other)
    }

    /// Whether two Unix-millisecond times can be the same instant stored at
    /// this precision (closer than one step; exact for `Unknown`). A time set
    /// on such a target is rounded to a step in either direction.
    pub fn same_instant(self, a_ms: i64, b_ms: i64) -> bool {
        let distance = a_ms.abs_diff(b_ms);
        match self.step_ms() {
            Some(step) => distance < step,
            None => distance == 0,
        }
    }

    /// Times stored as local time: a daylight-saving change can shift every
    /// reported time by exactly one hour (FAT).
    pub const fn local_time_shifts(self) -> bool {
        matches!(self, Self::TwoSeconds)
    }
}

/// Longest storable name component of a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameLimit {
    /// UTF-8 bytes (ext4, btrfs, xfs, f2fs, Android storage: 255).
    Bytes(usize),
    /// UTF-16 code units (NTFS, exFAT, FAT32 long names, SMB: 255).
    Utf16Units(usize),
}

impl NameLimit {
    pub fn fits(self, name: &str) -> bool {
        match self {
            Self::Bytes(limit) => name.len() <= limit,
            Self::Utf16Units(limit) => name.encode_utf16().count() <= limit,
        }
    }
}

/// Why a name cannot be stored on a target (an omission „Name auf Ziel
/// nicht möglich“, never an error per run).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameIssue {
    /// Windows naming rules forbid it; a `:` would even turn into an
    /// alternate data stream of another file on NTFS.
    Windows(Win32NameIssue),
    /// Longer than the target's name limit.
    TooLong,
}

/// What the filesystem or protocol below a root can store, so planning can
/// report what it cannot write instead of failing on it in every run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetLimits {
    /// Windows naming rules apply (NTFS/FAT/exFAT, SMB shares, Windows hosts).
    pub windows_names: bool,
    /// `None` = not known.
    pub max_name: Option<NameLimit>,
    /// Largest storable file in bytes (FAT32: 4 GiB − 1); `None` = not known.
    pub max_file_size: Option<u64>,
    pub mtime_precision: MtimePrecision,
}

impl TargetLimits {
    /// The reason `name` cannot be one path component here, if any.
    pub fn name_issue(&self, name: &str) -> Option<NameIssue> {
        if self.windows_names {
            if let Some(issue) = win32_name_issue(name) {
                return Some(NameIssue::Windows(issue));
            }
        }
        match self.max_name {
            Some(limit) if !limit.fits(name) => Some(NameIssue::TooLong),
            _ => None,
        }
    }

    /// Whether a file of `size` bytes can be stored here.
    pub fn fits_size(&self, size: u64) -> bool {
        self.max_file_size.is_none_or(|limit| size <= limit)
    }
}

/// How durable a finished stage must be before it is published.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StageDurability {
    /// No flush (copy/paste keeps Explorer semantics).
    #[default]
    NotRequired,
    /// Durable once `sync_filesystem` returned `Ok(true)` for a root above
    /// it; a backend flushes at once where that call cannot cover the stage
    /// (new files of a sync run).
    Deferred,
    /// Durable when `finish_stage` returns (a replacement of an existing file).
    Now,
}

/// What `finish_stage` does to a complete, closed, still unpublished stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageFinish {
    /// Source modification time (Unix ms) the stage should carry.
    pub mtime_ms: Option<i64>,
    /// Unix permission bits (masked to `0o777`) where the target keeps Unix
    /// modes; ignored elsewhere. Callers pass the source mode, made no more
    /// open than a destination it replaces (`source & destination`).
    pub mode: Option<u32>,
    pub durability: StageDurability,
}

/// What `finish_stage` achieved. Publishing (`promote_*`) keeps both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageFinished {
    /// The stage carries the requested time at `mtime_precision`, set now or
    /// already while uploading (`open_write_copy_stage_timed`). False: this
    /// target keeps its own times (compare it with its baseline instead).
    pub mtime_applied: bool,
    /// The requested durability holds. False: the target offers no such
    /// guarantee (always false for `StageDurability::NotRequired`).
    pub durable: bool,
}

/// What one host-side hash walk (`hash_walk`) returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashWalkRequest {
    /// Digest of every included regular file; `None` = size and time only.
    pub algorithm: Option<crate::analytics::HashAlgorithm>,
    /// Regular files smaller than this are left out (duplicate search); a
    /// sync walk passes 0, since a missing file would read as absent.
    pub min_bytes: u64,
}

/// One folder or regular file of a hash walk (path relative to its root).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashWalkEntry {
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
    /// Lowercase hex digest of the requested algorithm (files only).
    pub digest: Option<String>,
}

/// One streamed hash-walk item: an entry or an omission (links, special
/// files, unreadable or vanished entries, unrepresentable names).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HashWalkItem {
    Entry(HashWalkEntry),
    Omitted(VfsOmission),
}

/// The content a recycle request expects at its path (from a duplicate
/// search); the host re-checks it before moving anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleExpectation {
    pub size: u64,
    /// Lowercase hex SHA-256 of the whole content; `None` = size only.
    pub sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecycleOutcome {
    /// Moved to the trash of the device that stores it.
    Recycled,
    /// The content no longer matches the expectation; nothing was moved.
    Changed,
}

/// How a backend learns about changes of the other side (shown to users as
/// „Ereignisse“ or „Abfrage“).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeSignalMode {
    /// The other side pushes notices (Share `watch_v1`).
    Push,
    /// The backend asks a cheap cursor at the subscriber's interval
    /// (WebDAV root ETag, Drive change feed).
    Poll,
}

/// One notice of a change subscription. Notices only trigger runs; what
/// changed is still decided by the sync engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeNotice {
    /// The subscription is live: later changes are reported. What happened
    /// before (or since an earlier subscription ended) is unknown, so the
    /// subscriber checks the root once.
    Ready { generation: Option<u64> },
    /// Something below the root changed. `generation` grows with every change
    /// where the source counts them; `paths` (relative to the root) are hints.
    Changed {
        generation: Option<u64>,
        paths: Vec<String>,
    },
    /// Notices were lost (host overflow, restart, expired cursor): check all.
    Overflow,
    /// The subscription ended (connection lost, capability gone); poll and
    /// subscribe again later.
    Ended(String),
}

/// A running change subscription; dropping it stops the subscription.
pub struct ChangeSubscription {
    _guard: Box<dyn std::any::Any + Send>,
}

impl ChangeSubscription {
    /// `guard` ends the subscription when dropped (stop flag, joined thread,
    /// closed stream).
    pub fn new(guard: impl Send + 'static) -> Self {
        Self {
            _guard: Box::new(guard),
        }
    }
}

impl std::fmt::Debug for ChangeSubscription {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ChangeSubscription")
    }
}
