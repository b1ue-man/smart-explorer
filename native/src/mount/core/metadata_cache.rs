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
#[path = "metadata_refresh_order.rs"]
mod refresh_order;
#[path = "metadata_preload_records.rs"]
mod preload_records;
#[path = "metadata_preload.rs"]
mod preload;
#[path = "metadata_snapshot.rs"]
mod snapshot;
pub(super) use preload::PreloadTicket;
#[cfg(test)]
#[path = "vault_metadata_task_tests.rs"]
mod vault_task_tests;
#[cfg(test)]
#[path = "vault_metadata_flight_tests.rs"]
mod vault_flight_tests;
#[cfg(test)]
#[path = "bulk_metadata_task_tests.rs"]
mod bulk_task_tests;
use load_support::{expire_observed_path, invalidate_descendants, invalidate_paths,
    invalidate_slot, LoadTable, RetiredMetadata};
pub(super) use load_support::{Admission, DirectoryObservation, LoadSlot, MetadataLookup,
    SnapshotPublication};
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
    // A successful newer observation disproved this image, but it remains a
    // charged comparison baseline until change/admission pressure is resolved.
    comparison_only: bool,
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
    refresh: refresh_order::RefreshOrder,
    preload: preload_records::PreloadRecords,
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
        if cached.comparison_only || cached.listing_expires_at <= now {
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

    pub(super) fn invalidate(&self, path: &str, recursive: bool) -> io::Result<()> {
        let key = self.key(path);
        let prefix = format!("{}/", key.trim_end_matches('/'));
        let parent_key = parent_and_name(path).map(|(parent, _)| self.key(parent));
        let mut retired = RetiredMetadata::default();
        let mut loads = self.lock_loads()?;
        invalidate_paths(&mut loads, &key, &prefix, recursive, parent_key.as_deref(), &mut retired);
        let mut state = self.lock_state()?;
        state.generation = state.generation.saturating_add(1);
        let mut directory_keys = if recursive { order::descendants(&state.directories, &key) }
            else { Vec::new() };
        directory_keys.push(key.clone());
        for candidate in directory_keys {
            remove_directory(&mut state, &candidate, &mut retired);
        }
        let mut cooldowns = if recursive { order::descendants(&state.snapshot_cooldowns, &key) }
            else { Vec::new() };
        cooldowns.push(key.clone());
        for candidate in cooldowns { order::remove_cooldown(&mut state, &candidate); }
        if let Some(parent_key) = parent_key {
            remove_directory(&mut state, &parent_key, &mut retired);
            order::remove_cooldown(&mut state, &parent_key);
        }
        Ok(())
    }

    pub(super) fn cool_down_snapshot(&self, path: &str) -> io::Result<()> {
        let mut state = self.lock_state()?;
        self.cool_down_locked(&mut state, &self.key(path));
        Ok(())
    }

    pub(in crate::mount) fn cool_down_preload(&self, ticket: &PreloadTicket) -> io::Result<()> {
        let mut state = self.lock_state()?;
        // A failed old flight must not cool down a replacement parent's work.
        if preload::ticket_current(&state, ticket) {
            self.cool_down_locked(&mut state, &self.key(&ticket.path));
        }
        Ok(())
    }

    fn cool_down_locked(&self, state: &mut CacheState, key: &str) {
        let now = Instant::now();
        order::prune_cooldowns(state, now);
        preload::prune_retries(state, now, self.case_sensitive);
        order::remove_cooldown(state, key);
        // Retry bookkeeping is disposable too: bound its estimated bytes,
        // rather than imposing a directory count on valid mounted contents.
        let bytes = order::cooldown_bytes(key);
        let allowance = MAX_CACHED_BYTES.saturating_sub(state.bytes);
        let deadline = now + SNAPSHOT_RETRY_DELAY;
        if bytes <= allowance {
            while state.cooldown_bytes.saturating_add(bytes) > allowance {
                let Some((_, oldest)) = state.cooldown_expiry.first().cloned() else { break; };
                order::remove_cooldown(state, &oldest);
            }
            state.cooldown_bytes = state.cooldown_bytes.saturating_add(bytes);
            state.snapshot_cooldowns.insert(key.to_string(), deadline);
            state.cooldown_expiry.insert((deadline, key.to_string()));
            order::cool_down(state, key);
        }
        // Already charged parent records retain their retry even when pressure
        // cannot admit a standalone hint; otherwise the worker could hot-loop.
        preload::cool_down_child(state, key, deadline);
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
        Ok((!cached.comparison_only && cached.listing_expires_at <= Instant::now())
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
        let mut retired = RetiredMetadata::default();
        let mut loads = self.lock_loads()?;
        invalidate_slot(&mut loads, &key, &mut retired);
        invalidate_descendants(&mut loads, &key, &mut retired);
        if let Some(parent_key) = parent_key.as_ref() {
            invalidate_slot(&mut loads, parent_key, &mut retired);
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
