use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Weak};
use std::time::Instant;
use super::{support::parent_and_name, CacheState, MetadataCache};

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
}

impl LoadSlot {
    pub(super) fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            revision: AtomicU64::new(0),
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
}

pub(super) fn retain_active_loads(loads: &mut HashMap<String, Weak<LoadSlot>>) {
    loads.retain(|_, slot| slot.strong_count() > 0);
}

pub(super) fn invalidate_slot(loads: &mut HashMap<String, Weak<LoadSlot>>, key: &str) {
    retain_active_loads(loads);
    if let Some(slot) = loads.get(key).and_then(Weak::upgrade) {
        slot.invalidate();
    }
}

pub(super) fn invalidate_descendants(loads: &mut HashMap<String, Weak<LoadSlot>>, parent: &str) {
    retain_active_loads(loads);
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    for (candidate, slot) in loads.iter() {
        if candidate != parent && candidate.starts_with(&prefix) {
            if let Some(slot) = slot.upgrade() {
                slot.invalidate();
            }
        }
    }
}

pub(super) fn invalidate_paths(
    loads: &mut HashMap<String, Weak<LoadSlot>>,
    key: &str,
    prefix: &str,
    recursive: bool,
    parent: Option<&str>,
) {
    retain_active_loads(loads);
    for (candidate, slot) in loads.iter() {
        let affected = candidate == key
            || (recursive && candidate.starts_with(prefix))
            || parent == Some(candidate.as_str());
        if affected {
            if let Some(slot) = slot.upgrade() {
                slot.invalidate();
            }
        }
    }
}

pub(super) fn expire_observed_path(state: &mut CacheState, key: &str, parent: Option<&str>) {
    let prefix = format!("{}/", key.trim_end_matches('/'));
    let now = Instant::now();
    for (candidate, cached) in &mut state.directories {
        if candidate == key || candidate.starts_with(&prefix) || Some(candidate.as_str()) == parent {
            // Retain comparison images, but a newer exact observation must
            // supersede both containing-directory and descendant authority.
            cached.listing_expires_at = now;
            cached.metadata_expires_at = now;
        }
    }
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
        invalidate_descendants(&mut loads, &key);
        if let Some(parent) = &parent {
            invalidate_slot(&mut loads, parent);
        }
        expire_observed_path(&mut state, &key, parent.as_deref());
        Ok(true)
    }
}
