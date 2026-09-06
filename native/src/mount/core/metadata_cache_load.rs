use crate::vfs::VfsMeta;
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use super::{order, support::parent_and_name, CacheState, MetadataCache};

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

pub(super) fn invalidate_slot(loads: &mut LoadTable, key: &str) {
    if let Some(slot) = loads.slots.get(key).and_then(Weak::upgrade) {
        slot.invalidate();
    }
}

pub(super) fn invalidate_descendants(loads: &mut LoadTable, parent: &str) {
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    for (_, slot) in loads.slots.range(prefix.clone()..)
        .take_while(|(candidate, _)| candidate.starts_with(&prefix))
    {
        if let Some(slot) = slot.upgrade() { slot.invalidate(); }
    }
}

pub(super) fn invalidate_paths(
    loads: &mut LoadTable,
    key: &str,
    _prefix: &str,
    recursive: bool,
    parent: Option<&str>,
) {
    invalidate_slot(loads, key);
    if recursive { invalidate_descendants(loads, key); }
    if let Some(parent) = parent { invalidate_slot(loads, parent); }
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
        let mut loads = self.lock_loads()?;
        let mut state = self.lock_state()?;
        if slot.revision() != revision {
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
        invalidate_slot(&mut loads, &key);
        invalidate_descendants(&mut loads, &key);
        if let Some(parent) = &parent {
            invalidate_slot(&mut loads, parent);
        }
        expire_observed_path(&mut state, &key, parent.as_deref());
        Ok(true)
    }
}
