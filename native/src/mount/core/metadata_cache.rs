use super::case_semantics::identity_key;
use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

#[path = "metadata_cache_load.rs"]
mod load_support;
#[path = "metadata_cache_support.rs"]
mod support;
#[path = "metadata_changes.rs"]
mod changes;
#[path = "metadata_schedule.rs"]
mod schedule;
use load_support::{expire_observed_path, invalidate_descendants, invalidate_paths, invalidate_slot};
pub(super) use load_support::{Admission, DirectoryObservation, LoadSlot, MetadataLookup};
pub(super) use schedule::run_metadata_batch;
pub use changes::MetadataChange;
use support::*;

const MAX_CACHED_DIRECTORIES: usize = 4_096;
pub(super) const MAX_CACHED_ENTRIES: usize = 50_000;
const MAX_CACHED_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHED_DIRECTORY_BYTES: usize = MAX_CACHED_BYTES;
const SNAPSHOT_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
pub(super) const DIRECTORY_TTL: Duration = Duration::from_secs(20);

#[derive(Clone)]
struct CachedDirectory {
    path: String,
    metadata: VfsMeta,
    metadata_expires_at: Instant,
    entries: Arc<[VfsMeta]>,
    listing_expires_at: Instant,
    entry_index: Arc<HashMap<String, usize>>,
    depth: u8,
    entry_count: usize,
    byte_count: usize,
    last_touch: u64,
    last_access: u64,
    refreshed_through_access: u64,
    revision: u64,
    last_attempt: u64,
    deferred_changes: bool,
}

#[derive(Default)]
struct CacheState {
    directories: HashMap<String, CachedDirectory>,
    snapshot_cooldowns: HashMap<String, Instant>,
    entries: usize,
    bytes: usize,
    clock: u64,
    generation: u64,
    changes: changes::ChangeQueue,
}

pub(super) struct MetadataCache {
    root: String,
    case_sensitive: bool,
    state: Mutex<CacheState>,
    loads: Mutex<HashMap<String, Weak<LoadSlot>>>,
}

impl MetadataCache {
    pub(super) fn new(root: &str, case_sensitive: bool) -> Self {
        Self {
            root: root.to_string(),
            case_sensitive,
            state: Mutex::new(CacheState::default()),
            loads: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn stat(&self, path: &str) -> io::Result<MetadataLookup> {
        let mut state = self.lock_state()?;
        Ok(lookup_metadata(&mut state, path, self.case_sensitive))
    }

    pub(super) fn metadata_hint(&self, path: &str) -> io::Result<Option<(VfsMeta, Instant)>> {
        let mut state = self.lock_state()?;
        let (lookup, expires_at) = lookup_metadata_at(
            &mut state, path, self.case_sensitive, Instant::now(), false,
        );
        match lookup {
            MetadataLookup::Found(metadata) => Ok(Some((metadata, expires_at))),
            MetadataLookup::KnownMissing => Err(io::Error::new(
                io::ErrorKind::NotFound, "mounted metadata path does not exist",
            )),
            MetadataLookup::Uncached => Ok(None),
        }
    }

    pub(super) fn drain_changes(&self, limit: usize) -> io::Result<Vec<MetadataChange>> {
        Ok(self.lock_state()?.changes.drain(limit))
    }

    pub(super) fn directory(&self, path: &str) -> io::Result<Option<Arc<[VfsMeta]>>> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let now = Instant::now();
        match lookup_metadata_at(&mut state, path, self.case_sensitive, now, true).0 {
            MetadataLookup::KnownMissing => return Ok(None),
            MetadataLookup::Found(metadata) if !metadata.is_dir || metadata.is_symlink => {
                return Ok(None);
            }
            _ => {}
        }
        let touch = tick(&mut state);
        let Some(cached) = state.directories.get_mut(&key) else {
            return Ok(None);
        };
        cached.last_touch = touch;
        cached.last_access = touch;
        if cached.listing_expires_at <= now {
            return Ok(None);
        }
        let entries = Arc::clone(&cached.entries);
        Ok(Some(entries))
    }

    pub(super) fn mark_directory_access(&self, path: &str) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let touch = tick(&mut state);
        if let Some(cached) = state.directories.get_mut(&key) {
            cached.last_touch = touch;
            cached.last_access = touch;
        }
        Ok(())
    }

    pub(super) fn install_directory(
        &self,
        path: &str,
        metadata: VfsMeta,
        entries: Arc<[VfsMeta]>,
        depth: u8,
    ) -> io::Result<bool> {
        let expires_at = Instant::now() + DIRECTORY_TTL;
        self.install_observation(path, DirectoryObservation {
            metadata, metadata_expires_at: expires_at, entries,
            listing_expires_at: expires_at,
        }, depth, None, Admission::Demand)
    }

    pub(super) fn install_directory_if_current(
        &self,
        path: &str,
        metadata: VfsMeta,
        entries: Arc<[VfsMeta]>,
        depth: u8,
        slot: &LoadSlot,
        revision: u64,
    ) -> io::Result<bool> {
        let expires_at = Instant::now() + DIRECTORY_TTL;
        self.install_observation(path, DirectoryObservation {
            metadata, metadata_expires_at: expires_at, entries,
            listing_expires_at: expires_at,
        }, depth, Some((slot, revision)), Admission::Demand)
    }

    pub(super) fn install_observation(
        &self,
        path: &str,
        observation: DirectoryObservation,
        depth: u8,
        admission: Option<(&LoadSlot, u64)>,
        intent: Admission,
    ) -> io::Result<bool> {
        let DirectoryObservation { metadata, metadata_expires_at,
            entries, listing_expires_at } = observation;
        let entry_count = entries.len().saturating_add(1);
        let metadata_bytes = path
            .len()
            .saturating_mul(2)
            .saturating_add(meta_bytes(&metadata))
            .saturating_add(entries.iter().fold(0usize, |total, metadata| {
                total.saturating_add(meta_bytes(metadata))
            }));
        if entry_count > MAX_CACHED_ENTRIES || metadata_bytes > MAX_CACHED_DIRECTORY_BYTES {
            return Ok(false);
        }
        let (entry_index, index_bytes) = build_entry_index(&entries, self.case_sensitive)?;
        let byte_count = metadata_bytes
            .saturating_add(index_bytes)
            .saturating_add(std::mem::size_of::<CachedDirectory>());
        if byte_count > MAX_CACHED_DIRECTORY_BYTES {
            return Ok(false);
        }
        let key = self.key(path);
        let root_key = self.key(&self.root);
        let mut loads = self.lock_loads()?;
        let mut state = self.lock_state()?;
        if admission.is_some_and(|(slot, revision)| slot.revision() != revision) {
            return Ok(false);
        }
        let previous = state.directories.get(&key).cloned();
        let prepared_change = if let Some(previous) = &previous {
            let prepared = state.changes.prepare(
                path,
                changes::SnapshotImage { entries: Arc::clone(&previous.entries),
                    index: Arc::clone(&previous.entry_index), bytes: previous.byte_count },
                changes::SnapshotImage { entries: Arc::clone(&entries),
                    index: Arc::clone(&entry_index), bytes: byte_count },
                self.case_sensitive,
            );
            let Some(prepared) = prepared else {
                // Keep this comparison baseline even if an unrelated demand
                // needs cache space before notification pressure clears.
                if let Some(cached) = state.directories.get_mut(&key) {
                    cached.deferred_changes = true;
                }
                return Ok(false);
            };
            Some(prepared)
        } else {
            None
        };
        let last_access = previous.as_ref().map_or(0, |cached| cached.last_access);
        // Subtract a replacement before calculating pressure. Speculation and
        // maintenance never evict another snapshot, including a demanded one.
        remove_directory(&mut state, &key);
        if intent == Admission::Demand {
            evict_until(
                &mut state,
                entry_count,
                byte_count,
                &root_key,
                Some(&key),
                true,
            );
        }
        if !fits(&state, entry_count, byte_count)
            || state.directories.len() >= MAX_CACHED_DIRECTORIES
        {
            if let Some(previous) = previous {
                restore_directory(&mut state, key, previous);
            }
            return Ok(false);
        }
        invalidate_descendants(&mut loads, &key);
        // A refresh releases its fetch guard before taking namespace authority.
        // Any intervening same-path install must also reject that older result.
        invalidate_slot(&mut loads, &key);
        let last_touch = tick(&mut state);
        if let Some(prepared) = prepared_change {
            state.changes.commit(prepared);
        }
        state.generation = state.generation.saturating_add(1);
        state.entries += entry_count;
        state.bytes += byte_count;
        state.directories.insert(
            key.clone(),
            CachedDirectory {
                path: path.to_string(),
                metadata,
                metadata_expires_at,
                entries: Arc::clone(&entries),
                listing_expires_at,
                entry_index,
                depth,
                entry_count,
                byte_count,
                last_touch,
                last_access,
                refreshed_through_access: last_access,
                revision: last_touch,
                last_attempt: last_touch,
                deferred_changes: false,
            },
        );
        state.snapshot_cooldowns.remove(&key);
        reconcile_direct_children(
            &mut state, &key, &entries, self.case_sensitive,
            previous.as_ref().map(|previous| (previous.entries.as_ref(), previous.entry_index.as_ref())),
        );
        Ok(true)
    }

    pub(super) fn invalidate(&self, path: &str, recursive: bool) -> io::Result<()> {
        let key = self.key(path);
        let prefix = format!("{}/", key.trim_end_matches('/'));
        let parent_key = parent_and_name(path).map(|(parent, _)| self.key(parent));
        let mut loads = self.lock_loads()?;
        invalidate_paths(&mut loads, &key, &prefix, recursive, parent_key.as_deref());
        let mut state = self.lock_state()?;
        state.generation = state.generation.saturating_add(1);
        let directory_keys = state
            .directories
            .keys()
            .filter(|candidate| *candidate == &key || (recursive && candidate.starts_with(&prefix)))
            .cloned()
            .collect::<Vec<_>>();
        for candidate in directory_keys {
            remove_directory(&mut state, &candidate);
        }
        state.snapshot_cooldowns.retain(|candidate, _| {
            candidate != &key
                && !(recursive && candidate.starts_with(&prefix))
                && parent_key.as_ref() != Some(candidate)
        });
        if let Some(parent_key) = parent_key {
            remove_directory(&mut state, &parent_key);
        }
        Ok(())
    }

    pub(super) fn cool_down_snapshot(&self, path: &str) -> io::Result<()> {
        let key = self.key(path);
        let mut state = self.lock_state()?;
        if state.snapshot_cooldowns.len() >= MAX_CACHED_DIRECTORIES
            && !state.snapshot_cooldowns.contains_key(&key)
        {
            if let Some(victim) = state
                .snapshot_cooldowns
                .iter()
                .min_by_key(|(_, retry_at)| **retry_at)
                .map(|(path, _)| path.clone())
            {
                state.snapshot_cooldowns.remove(&victim);
            }
        }
        state
            .snapshot_cooldowns
            .insert(key, Instant::now() + SNAPSHOT_RETRY_DELAY);
        Ok(())
    }

    pub(super) fn load_slot(&self, path: &str) -> io::Result<Arc<LoadSlot>> {
        let key = self.key(path);
        let mut loads = self.lock_loads()?;
        load_support::retain_active_loads(&mut loads);
        if let Some(slot) = loads.get(&key).and_then(Weak::upgrade) {
            return Ok(slot);
        }
        let slot = Arc::new(LoadSlot::new());
        loads.insert(key, Arc::downgrade(&slot));
        Ok(slot)
    }

    pub(super) fn revision(&self, path: &str) -> io::Result<Option<u64>> {
        let state = self.lock_state()?;
        Ok(state
            .directories
            .get(&self.key(path))
            .map(|entry| entry.revision))
    }

    pub(super) fn generation(&self) -> io::Result<u64> {
        Ok(self.lock_state()?.generation)
    }

    pub(super) fn note_external_observation(&self) -> io::Result<()> {
        let mut state = self.lock_state()?;
        state.generation = state.generation.saturating_add(1);
        Ok(())
    }

    pub(super) fn note_path_observation(&self, path: &str) -> io::Result<()> {
        let parent_key = parent_and_name(path).map(|(parent, _)| self.key(parent));
        let key = self.key(path);
        let mut loads = self.lock_loads()?;
        invalidate_descendants(&mut loads, &key);
        if let Some(parent_key) = parent_key.as_ref() {
            invalidate_slot(&mut loads, parent_key);
        }
        let mut state = self.lock_state()?;
        expire_observed_path(&mut state, &key, parent_key.as_deref());
        Ok(())
    }

    pub(super) fn validate_listing(&self, entries: &[VfsMeta]) -> io::Result<()> {
        validate_listing(entries)
    }

    #[cfg(test)]
    pub(super) fn usage(&self) -> io::Result<(usize, usize, usize)> {
        let state = self.lock_state()?;
        Ok((state.directories.len(), state.entries, state.bytes))
    }

    #[cfg(test)]
    pub(super) fn cooldown_count(&self) -> io::Result<usize> {
        Ok(self.lock_state()?.snapshot_cooldowns.len())
    }

    fn key(&self, value: &str) -> String {
        identity_key(self.case_sensitive, value)
    }

    fn lock_state(&self) -> io::Result<std::sync::MutexGuard<'_, CacheState>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("mount metadata cache is unavailable"))
    }

    fn lock_loads(&self) -> io::Result<MutexGuard<'_, HashMap<String, Weak<LoadSlot>>>> {
        self.loads
            .lock()
            .map_err(|_| io::Error::other("metadata load table is unavailable"))
    }
}
