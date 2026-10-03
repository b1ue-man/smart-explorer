//! Per-run, per-replica versions with target-side rename and private fallback.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use super::pair_lock::PairLock;
use super::persistence::versions_dir;
use super::run_types::StateOwner;
use super::types::{PairSide, Versioning, VersionsLocation};
use crate::vfs::Backend;

/// What the versions of one run need to know.
#[derive(Clone, Debug)]
pub struct VersionsContext {
    pub pair_id: String,
    pub owner: StateOwner,
    /// Job name for the readable directory ("" for syncs without a job).
    pub job_name: String,
    /// Folder of this run below `.se-versions` ([`new_run_id`]).
    pub run_id: String,
    /// Unix milliseconds the run started.
    pub started_ms: i64,
    pub location: VersionsLocation,
    /// Retention, counted per file.
    pub versioning: Versioning,
}

impl VersionsContext {
    pub fn new(
        pair_id: &str,
        owner: StateOwner,
        location: VersionsLocation,
        versioning: Versioning,
    ) -> Self {
        let started_ms = now_ms();
        Self {
            pair_id: pair_id.to_string(),
            owner,
            job_name: String::new(),
            run_id: new_run_id(started_ms),
            started_ms,
            location,
            versioning,
        }
    }
}

/// Sortable, unique run id: the start in UTC plus a random suffix, e.g.
/// `20261002T153012Z-1f2e3d4c`.
pub fn new_run_id(started_ms: i64) -> String {
    let stamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(started_ms)
        .map(|time| time.format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|| started_ms.to_string());
    let mut random = [0u8; 4];
    let suffix = match getrandom::getrandom(&mut random) {
        Ok(()) => u32::from_be_bytes(random),
        Err(_) => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or(0),
    };
    format!("{stamp}-{suffix:08x}")
}

/// The versions of one run, on both sides. The orchestration begins it before
/// apply, hands it to apply (`ApplyScope::versions`) and finishes it after
/// apply, also after errors and cancels.
#[derive(Debug)]
pub struct RunVersions {
    context: VersionsContext,
    app_data: PathBuf,
    lock_id: std::sync::Mutex<String>,
}

impl RunVersions {
    pub fn begin(context: VersionsContext) -> Self {
        let app_data = versions_dir(&context.pair_id);
        Self {
            context,
            app_data,
            lock_id: std::sync::Mutex::new(String::new()),
        }
    }

    pub fn context(&self) -> &VersionsContext {
        &self.context
    }

    pub fn run_id(&self) -> &str {
        &self.context.run_id
    }

    /// The pair's app-data store (`versions_<pair>`): the location before RV1
    /// and the fallback where no rename is possible.
    pub fn app_data_dir(&self) -> &Path {
        &self.app_data
    }

    /// Each version's readable immutable record is durable before destructive
    /// work starts. There is no pending backend reference or unflushed index.
    pub fn finish(&self) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn bind_lock(&self, id: &str) -> io::Result<()> {
        let mut current = self.lock_id.lock().unwrap_or_else(|e| e.into_inner());
        if !current.is_empty() && *current != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "versions belong to another pair lock",
            ));
        }
        *current = id.to_string();
        Ok(())
    }
    pub(super) fn lock_id(&self) -> String {
        self.lock_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(super) fn with_app_data(context: VersionsContext, app_data: PathBuf) -> Self {
        Self {
            context,
            app_data,
            lock_id: std::sync::Mutex::new(String::new()),
        }
    }
}

/// Why a version was kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionReason {
    /// Replaced by a newer copy from the other side.
    Replaced,
    /// Deleted because the other side deleted it (or a mirror orphan).
    Deleted,
    /// Lost a conflict resolution.
    Resolved,
    /// Replaced by restoring an older version.
    Restored,
}

/// Where a version is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionStore {
    /// `.se-versions` at the side's sync root.
    SyncRoot,
    /// The app data of this device (`versions_<pair>`).
    AppData,
}

/// One kept version, as the readable directory lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionEntry {
    /// Side the file belonged to (`None`: app-data versions from before RV1).
    pub side: Option<PairSide>,
    /// Original path relative to the sync root.
    pub rel: String,
    pub run_id: String,
    /// Unix milliseconds the version was kept.
    pub preserved_ms: i64,
    /// `None`: versions from before RV1.
    pub reason: Option<VersionReason>,
    pub size: u64,
    pub mtime_ms: i64,
    pub store: VersionStore,
    /// The kept copy: a path of the side's backend (`SyncRoot`) or a local
    /// path (`AppData`).
    pub stored_path: String,
    pub job_id: Option<String>,
}

/// One side the versions functions may look at.
pub struct VersionSide<'a> {
    pub side: PairSide,
    pub backend: &'a dyn Backend,
    pub root: &'a str,
}

/// Prunes the pair's versions after a run, per file (Y38/Y56), on the given
/// sides' `.se-versions` and in the app data, independently per file/replica.
pub fn prune_after_run(
    lock: &PairLock,
    pair_id: &str,
    sides: &[VersionSide<'_>],
    versioning: &Versioning,
    cancel: &AtomicBool,
) -> io::Result<()> {
    super::version_ops::prune(lock, pair_id, sides, versioning, cancel)
}

/// Lists the pair's kept versions, newest first (desktop and Android
/// "Versionen…"). Foreign pair and replica records are never mutated.
pub fn list_versions(
    pair_id: &str,
    sides: &[VersionSide<'_>],
    cancel: &AtomicBool,
) -> io::Result<Vec<VersionEntry>> {
    super::version_ops::list(pair_id, sides, cancel)
}

/// Restores `entry` to its original path on `side`, keeping the file it
/// replaces as a version (`VersionReason::Restored`). The next run carries the
/// restored file to the other side; the old baseline remains a comparison.
pub fn restore_version(
    lock: &PairLock,
    pair_id: &str,
    entry: &VersionEntry,
    side: &VersionSide<'_>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    super::version_restore::restore(lock, pair_id, entry, side, cancel)
}

/// Removes every kept version of the pair on the given sides and in the app
/// data (offered when a job is deleted or retargeted, Y150), preserving
/// foreign/link-like entries and unknown children.
pub fn remove_versions(
    lock: &PairLock,
    pair_id: &str,
    sides: &[VersionSide<'_>],
    cancel: &AtomicBool,
) -> io::Result<()> {
    super::version_ops::remove(lock, pair_id, sides, cancel)
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
