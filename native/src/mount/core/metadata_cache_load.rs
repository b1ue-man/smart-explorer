use crate::vfs::VfsMeta;
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use super::{order, support::{lookup_metadata_at, parent_and_name},
    CacheState, CachedDirectory, MetadataCache};

pub(in crate::mount) enum MetadataLookup {
    Found(VfsMeta),
    KnownMissing,
    Uncached,
}

pub(in crate::mount) struct DirectoryObservation {
    pub metadata: VfsMeta,
    pub metadata_expires_at: Instant,
    pub entries: std::sync::Arc<[VfsMeta]>,
    pub listing_expires_at: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::mount) enum Admission {
    Demand,
    Refresh,
    Speculative,
}

#[derive(Clone, Copy)]
pub(in crate::mount) struct SnapshotPublication {
    pub(in crate::mount) retained: bool,
    // Captured at publication, never inferred from a later revision reread.
    pub(in crate::mount) completed_revision: Option<u64>,
}

impl SnapshotPublication {
    pub(super) fn obsolete() -> Self { Self { retained: false, completed_revision: None } }
}

/// Declare before acquiring global guards. It also owns temporary strong slot
/// upgrades: their final drop can release a completed wide listing.
#[derive(Default)]
pub(super) struct RetiredMetadata {
    directories: Vec<CachedDirectory>,
    slots: Vec<Arc<LoadSlot>>,
    completed: Vec<CompletedDirectory>,
    changes: Vec<super::changes::PendingDiff>,
}

impl RetiredMetadata {
    pub(super) fn directory(&mut self, directory: Option<CachedDirectory>) {
        if let Some(directory) = directory { self.directories.push(directory); }
    }

    pub(super) fn diff(&mut self, diff: Option<super::changes::PendingDiff>) {
        if let Some(diff) = diff { self.changes.push(diff); }
    }
}

pub(in crate::mount) struct LoadSlot {
    gate: Mutex<()>,
    revision: AtomicU64,
    completed: Mutex<Option<CompletedDirectory>>,
}

struct CompletedDirectory {
    revision: u64,
    expires_at: Instant,
    result: Result<Arc<[VfsMeta]>, SharedFailure>,
}

struct SharedFailure {
    kind: io::ErrorKind,
    raw: Option<i32>,
    message: String,
}

impl SharedFailure {
    fn error(&self) -> io::Error {
        self.raw.map(io::Error::from_raw_os_error)
            .unwrap_or_else(|| io::Error::new(self.kind, self.message.clone()))
    }
}

#[derive(Default)]
pub(super) struct LoadTable {
    slots: BTreeMap<String, Weak<LoadSlot>>,
    prune_cursor: Option<String>,
}

impl LoadTable {
    pub(super) fn slot(&mut self, key: String) -> Arc<LoadSlot> {
        // Amortized, bounded cleanup; never sweep all active load paths for
        // an unrelated lookup. A weak table does not retain completed images.
        use std::ops::Bound::{Excluded, Unbounded};
        let keys = match self.prune_cursor.as_ref() {
            Some(cursor) => self.slots.range((Excluded(cursor.clone()), Unbounded))
                .take(16).map(|(path, _)| path.clone()).collect::<Vec<_>>(),
            None => self.slots.keys().take(16).cloned().collect(),
        };
        self.prune_cursor = keys.last().cloned();
        for path in keys {
            if self.slots.get(&path).is_some_and(|slot| slot.strong_count() == 0) {
                self.slots.remove(&path);
            }
        }
        if let Some(slot) = self.slots.get(&key).and_then(Weak::upgrade) { return slot; }
        let slot = Arc::new(LoadSlot::new());
        self.slots.insert(key, Arc::downgrade(&slot));
        slot
    }
}

impl LoadSlot {
    pub(super) fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            revision: AtomicU64::new(0),
            completed: Mutex::new(None),
        }
    }

    pub(in crate::mount) fn lock(&self) -> io::Result<MutexGuard<'_, ()>> {
        self.gate
            .lock()
            .map_err(|_| io::Error::other("metadata load slot is unavailable"))
    }

    pub(in crate::mount) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(super) fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(super) fn discard_completed(&self, retired: &mut RetiredMetadata) -> io::Result<()> {
        let mut completed = self.completed.lock()
            .map_err(|_| io::Error::other("metadata load result is unavailable"))?;
        if let Some(completed) = completed.take() { retired.completed.push(completed); }
        Ok(())
    }

    pub(in crate::mount) fn completed_directory(&self) -> io::Result<Option<Arc<[VfsMeta]>>> {
        let completed = self.completed.lock()
            .map_err(|_| io::Error::other("metadata load result is unavailable"))?;
        match completed.as_ref().filter(|result| result.revision == self.revision()
            && result.expires_at > Instant::now())
        {
            Some(result) => result.result.as_ref().map(|entries| Some(Arc::clone(entries)))
                .map_err(SharedFailure::error),
            None => Ok(None),
        }
    }

    pub(in crate::mount) fn complete_directory(
        &self, revision: u64, expires_at: Instant, entries: Arc<[VfsMeta]>,
    ) -> io::Result<()> {
        let mut completed = self.completed.lock()
            .map_err(|_| io::Error::other("metadata load result is unavailable"))?;
        if self.revision() == revision && expires_at > Instant::now() {
            *completed = Some(CompletedDirectory { revision, expires_at, result: Ok(entries) });
        }
        Ok(())
    }

    /// Failure sharing lasts only while this same weak-table flight has owners;
    /// it is not persistent caching of permission/transport errors.
    pub(in crate::mount) fn complete_directory_failure(
        &self, revision: u64, error: &io::Error,
    ) -> io::Result<()> {
        let mut completed = self.completed.lock()
            .map_err(|_| io::Error::other("metadata load result is unavailable"))?;
        if self.revision() == revision {
            *completed = Some(CompletedDirectory { revision,
                expires_at: Instant::now() + Duration::from_secs(1),
                result: Err(SharedFailure { kind: error.kind(), raw: error.raw_os_error(),
                    message: error.to_string() }) });
        }
        Ok(())
    }
}

pub(super) fn invalidate_slot(loads: &mut LoadTable, key: &str, retired: &mut RetiredMetadata) {
    if let Some(slot) = loads.slots.get(key).and_then(Weak::upgrade) {
        slot.invalidate();
        retired.slots.push(slot);
    }
}

pub(super) fn invalidate_descendants(
    loads: &mut LoadTable, parent: &str, retired: &mut RetiredMetadata,
) {
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    for (_, slot) in loads.slots.range(prefix.clone()..)
        .take_while(|(candidate, _)| candidate.starts_with(&prefix))
        .filter(|(candidate, _)| candidate.as_str() != parent)
    {
        if let Some(slot) = slot.upgrade() { slot.invalidate(); retired.slots.push(slot); }
    }
}

pub(super) fn invalidate_direct_children(
    loads: &mut LoadTable, parent: &str, retired: &mut RetiredMetadata,
) {
    use std::ops::Bound::{Excluded, Included, Unbounded};
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    let mut lower = Included(prefix.clone());
    loop {
        let Some(path) = loads.slots.range((lower, Unbounded)).next()
            .map(|(path, _)| path.clone()) else { break; };
        if !path.starts_with(&prefix) { break; }
        if path == parent { lower = Excluded(path); continue; }
        if let Some((child, _)) = path[prefix.len()..].split_once('/') {
            // Canonical keys use '/' separators. '0' is its immediate ASCII
            // successor, so this jumps over exactly child + '/' descendants
            // while preserving a sibling named child + '0' (Included bound).
            lower = Included(format!("{prefix}{child}0"));
        } else {
            invalidate_slot(loads, &path, retired);
            lower = Excluded(path);
        }
    }
}

pub(super) fn publish_observation(
    loads: &mut LoadTable, key: &str, admission: Option<(&LoadSlot, u64)>,
    retired: &mut RetiredMetadata,
) -> io::Result<Option<u64>> {
    if let Some((slot, revision)) = admission {
        slot.discard_completed(retired)?;
        slot.invalidate();
        if let Some(indexed) = loads.slots.get(key).and_then(Weak::upgrade) {
            if !std::ptr::eq(indexed.as_ref(), slot) { indexed.invalidate(); }
            retired.slots.push(indexed);
        }
        Ok(Some(revision.wrapping_add(1)))
    } else {
        invalidate_slot(loads, key, retired);
        Ok(None)
    }
}

pub(super) fn invalidate_paths(
    loads: &mut LoadTable,
    key: &str,
    _prefix: &str,
    recursive: bool,
    parent: Option<&str>,
    retired: &mut RetiredMetadata,
) {
    invalidate_slot(loads, key, retired);
    if recursive { invalidate_descendants(loads, key, retired); }
    if let Some(parent) = parent { invalidate_slot(loads, parent, retired); }
}

pub(super) fn expire_observed_path(state: &mut CacheState, key: &str, parent: Option<&str>) {
    let now = Instant::now();
    let descendants = order::descendants(&state.directories, key);
    for candidate in descendants { order::expire(state, &candidate, now); }
    order::expire(state, key, now);
    if let Some(parent) = parent { order::expire(state, parent, now); }
    state.generation = state.generation.saturating_add(1);
}

impl MetadataCache {
    pub(in crate::mount) fn install_point_if_current(
        &self, path: &str, slot: &LoadSlot, revision: u64,
        points: &super::super::metadata_point_cache::MetadataPointCache,
        metadata: Option<VfsMeta>,
    ) -> io::Result<bool> {
        let mut retired = RetiredMetadata::default();
        let mut loads = self.lock_loads()?;
        let mut state = self.lock_state()?;
        if slot.revision() != revision {
            return Ok(false);
        }
        // A retained parent may have published after the caller's last lookup
        // without changing this unchanged child's revision. Recheck under the
        // same load/state locks as publication, before installing a point or
        // expiring snapshot authority. The caller then returns that authority.
        if !matches!(lookup_metadata_at(&mut state, path, self.case_sensitive,
            Instant::now(), false).0, MetadataLookup::Uncached)
        {
            return Ok(false);
        }
        // Lock order: load table -> snapshot state -> point state. Point
        // methods never acquire either snapshot lock; no backend I/O occurs.
        match metadata {
            Some(metadata) => points.install(path, metadata)?,
            None => points.install_missing(path)?,
        }
        let key = self.key(path);
        let parent = parent_and_name(path).map(|(parent, _)| self.key(parent));
        // An exact observation also supersedes a completed same-path listing
        // and any older refresh waiting to regain its installation guard.
        invalidate_slot(&mut loads, &key, &mut retired);
        invalidate_descendants(&mut loads, &key, &mut retired);
        if let Some(parent) = &parent {
            invalidate_slot(&mut loads, parent, &mut retired);
        }
        expire_observed_path(&mut state, &key, parent.as_deref());
        Ok(true)
    }
}
