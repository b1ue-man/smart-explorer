use super::case_semantics::identity_key;
use crate::vfs::VfsMeta;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[path = "metadata_cache_load.rs"]
mod load_support;
#[path = "metadata_cache_support.rs"]
mod support;
#[path = "metadata_changes.rs"]
mod changes;
#[path = "metadata_schedule.rs"]
mod schedule;
#[path = "metadata_cache_order.rs"]
mod order;
#[cfg(test)]
#[path = "vault_metadata_task_tests.rs"]
mod vault_task_tests;
#[cfg(test)]
#[path = "vault_metadata_flight_tests.rs"]
mod vault_flight_tests;
use load_support::{expire_observed_path, invalidate_descendants, invalidate_paths,
    invalidate_slot, LoadTable};
pub(super) use load_support::{Admission, DirectoryObservation, LoadSlot, MetadataLookup};
#[cfg(test)]
pub(super) use crate::mount::metadata_batch::run_metadata_batch;
pub use changes::MetadataChange;
use support::*;

// Kept only as a historical threshold for regression fixtures, not admission.
#[cfg(test)]
pub(super) const MAX_CACHED_ENTRIES: usize = 50_000;
const MAX_CACHED_BYTES: usize = 128 * 1024 * 1024;
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
    directories: BTreeMap<String, CachedDirectory>,
    recency: BTreeSet<(u64, String)>,
    expiry: BTreeSet<(Instant, String)>,
    snapshot_cooldowns: BTreeMap<String, Instant>,
    cooldown_expiry: BTreeSet<(Instant, String)>,
    cooldown_bytes: usize,
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
    loads: Mutex<LoadTable>,
}

impl MetadataCache {
    pub(super) fn new(root: &str, case_sensitive: bool) -> Self {
        Self {
            root: root.to_string(),
            case_sensitive,
            state: Mutex::new(CacheState::default()),
            loads: Mutex::new(LoadTable::default()),
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
        let (changes, retired) = {
            let mut state = self.lock_state()?;
            state.changes.drain(limit)
        };
        // Releasing a final snapshot Arc can free a wide directory's strings;
        // keep those destructors outside the foreground cache-state mutex.
        drop(retired);
        Ok(changes)
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
        order::touch(&mut state, &key, true);
        let Some(cached) = state.directories.get(&key) else {
            return Ok(None);
        };
        if cached.listing_expires_at <= now {
            return Ok(None);
        }
        let entries = Arc::clone(&cached.entries);
        Ok(Some(entries))
    }

    pub(super) fn mark_directory_access(&self, path: &str) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        order::touch(&mut state, &key, true);
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
        self.install_snapshot(path, observation, depth, admission, intent, None)
    }

    pub(super) fn install_observation_reconciled(
        &self, path: &str, observation: DirectoryObservation, depth: u8,
        admission: Option<(&LoadSlot, u64)>, intent: Admission,
        points: &super::metadata_point_cache::MetadataPointCache,
    ) -> io::Result<bool> {
        self.install_snapshot(path, observation, depth, admission, intent, Some(points))
    }

    fn install_snapshot(
        &self, path: &str, observation: DirectoryObservation, depth: u8,
        admission: Option<(&LoadSlot, u64)>, intent: Admission,
        points: Option<&super::metadata_point_cache::MetadataPointCache>,
    ) -> io::Result<bool> {
        let DirectoryObservation { metadata, metadata_expires_at,
            entries, listing_expires_at } = observation;
        let key = self.key(path);
        let entry_count = entries.len().saturating_add(1);
        let metadata_bytes = path.len()
            // Snapshot path, ordered-map key, recency/expiry keys and tree-node
            // bookkeeping are included, not merely the metadata payload.
            .saturating_add(key.capacity().saturating_mul(3))
            .saturating_add(256)
            .saturating_add(meta_bytes(&metadata))
            .saturating_add(entries.iter().fold(0usize, |total, metadata| {
                total.saturating_add(meta_bytes(metadata))
            }));
        if metadata_bytes > MAX_CACHED_DIRECTORY_BYTES {
            return Ok(false);
        }
        let (entry_index, index_bytes) = build_entry_index(&entries, self.case_sensitive)?;
        let byte_count = metadata_bytes
            .saturating_add(index_bytes)
            .saturating_add(std::mem::size_of::<CachedDirectory>());
        if byte_count > MAX_CACHED_DIRECTORY_BYTES {
            return Ok(false);
        }
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
                order::pin_changes(&mut state, &key);
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
        if !fits(&state, entry_count, byte_count) {
            if let Some(previous) = previous {
                restore_directory(&mut state, key, previous);
            }
            return Ok(false);
        }
        if let Some(points) = points {
            // The established order is load table -> snapshots -> points.
            // Reconcile identity-replaced subtrees before publishing their new
            // parent authority, without a window for an older point hit.
            if let Err(error) = points.reconcile_snapshot(path, &entries,
                previous.as_ref().map(|previous| previous.entries.as_ref()))
            {
                if let Some(previous) = previous { restore_directory(&mut state, key, previous); }
                return Err(error);
            }
        }
        reconcile_loads(&mut loads, &key, &entries, &entry_index,
            previous.as_ref().map(|previous| (previous.entries.as_ref(),
                previous.entry_index.as_ref())), self.case_sensitive);
        // A refresh releases its fetch guard before taking namespace authority.
        // Any intervening same-path install must also reject that older result.
        invalidate_slot(&mut loads, &key);
        let last_touch = tick(&mut state);
        let retired = prepared_change.and_then(|prepared| state.changes.commit(prepared));
        state.generation = state.generation.saturating_add(1);
        order::insert(&mut state,
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
        order::remove_cooldown(&mut state, &key);
        reconcile_direct_children(
            &mut state, &key, &entries, self.case_sensitive,
            previous.as_ref().map(|previous| (previous.entries.as_ref(), previous.entry_index.as_ref())),
        );
        drop(state);
        drop(loads);
        drop(previous);
        drop(retired);
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
        let mut directory_keys = if recursive { order::descendants(&state.directories, &key) }
            else { Vec::new() };
        directory_keys.push(key.clone());
        for candidate in directory_keys {
            remove_directory(&mut state, &candidate);
        }
        let mut cooldowns = if recursive { order::descendants(&state.snapshot_cooldowns, &key) }
            else { Vec::new() };
        cooldowns.push(key.clone());
        for candidate in cooldowns { order::remove_cooldown(&mut state, &candidate); }
        if let Some(parent_key) = parent_key {
            remove_directory(&mut state, &parent_key);
            order::remove_cooldown(&mut state, &parent_key);
        }
        Ok(())
    }

    pub(super) fn cool_down_snapshot(&self, path: &str) -> io::Result<()> {
        let key = self.key(path);
        let mut state = self.lock_state()?;
        let now = Instant::now();
        order::prune_cooldowns(&mut state, now);
        order::remove_cooldown(&mut state, &key);
        // Retry bookkeeping is disposable too: bound its estimated bytes,
        // rather than imposing a directory count on valid mounted contents.
        let bytes = order::cooldown_bytes(&key);
        let allowance = MAX_CACHED_BYTES.saturating_sub(state.bytes);
        if bytes <= allowance {
            while state.cooldown_bytes.saturating_add(bytes) > allowance {
                let Some((_, oldest)) = state.cooldown_expiry.first().cloned() else { break; };
                order::remove_cooldown(&mut state, &oldest);
            }
            let deadline = now + SNAPSHOT_RETRY_DELAY;
            state.cooldown_bytes = state.cooldown_bytes.saturating_add(bytes);
            state.snapshot_cooldowns.insert(key.clone(), deadline);
            state.cooldown_expiry.insert((deadline, key));
        }
        Ok(())
    }

    pub(super) fn load_slot(&self, path: &str) -> io::Result<Arc<LoadSlot>> {
        let key = self.key(path);
        let mut loads = self.lock_loads()?;
        Ok(loads.slot(key))
    }

    pub(super) fn revision(&self, path: &str) -> io::Result<Option<u64>> {
        let state = self.lock_state()?;
        Ok(state
            .directories
            .get(&self.key(path))
            .map(|entry| entry.revision))
    }

    /// Only a previously observed, now expired direct parent can be refreshed
    /// for stat coalescing. A cold stat never introduces an ancestor listing.
    pub(super) fn expired_parent(&self, path: &str) -> io::Result<Option<(String, u8)>> {
        let Some((parent, _)) = parent_and_name(path) else { return Ok(None); };
        let state = self.lock_state()?;
        let Some(cached) = state.directories.get(&self.key(parent)) else { return Ok(None); };
        Ok((cached.listing_expires_at <= Instant::now())
            .then(|| (cached.path.clone(), cached.depth)))
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
        invalidate_slot(&mut loads, &key);
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

    fn lock_loads(&self) -> io::Result<MutexGuard<'_, LoadTable>> {
        self.loads
            .lock()
            .map_err(|_| io::Error::other("metadata load table is unavailable"))
    }
}
