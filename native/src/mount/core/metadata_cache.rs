use super::case_semantics::identity_key;
use crate::vfs::VfsMeta;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

#[path = "metadata_cache_support.rs"]
mod support;
use support::*;

const MAX_CACHED_DIRECTORIES: usize = 4_096;
const MAX_CACHED_ENTRIES: usize = 50_000;
const MAX_CACHED_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHED_DIRECTORY_BYTES: usize = 4 * 1024 * 1024;
const SNAPSHOT_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
struct CachedDirectory {
    path: String,
    metadata: VfsMeta,
    entries: Arc<[VfsMeta]>,
    depth: u8,
    entry_count: usize,
    byte_count: usize,
    last_touch: u64,
    last_access: u64,
    refreshed_through_access: u64,
    revision: u64,
}

#[derive(Default)]
struct CacheState {
    directories: HashMap<String, CachedDirectory>,
    snapshot_cooldowns: HashMap<String, Instant>,
    entries: usize,
    bytes: usize,
    clock: u64,
    generation: u64,
    refresh_cursor: usize,
}

pub(super) struct MetadataCache {
    root: String,
    case_sensitive: bool,
    state: Mutex<CacheState>,
    loads: Mutex<HashMap<String, Weak<Mutex<()>>>>,
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

    pub(super) fn stat(&self, path: &str) -> io::Result<Option<VfsMeta>> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let touch = tick(&mut state);
        if let Some((parent, name)) = parent_and_name(path) {
            let parent_key = self.key(parent);
            let name_key = self.key(name);
            if let Some(cached) = state.directories.get_mut(&parent_key) {
                cached.last_touch = touch;
                cached.last_access = touch;
                return Ok(cached
                    .entries
                    .iter()
                    .find(|metadata| self.key(&metadata.name) == name_key)
                    .cloned());
            }
        }
        let Some(cached) = state.directories.get_mut(&key) else {
            return Ok(None);
        };
        cached.last_touch = touch;
        cached.last_access = touch;
        Ok(Some(cached.metadata.clone()))
    }

    pub(super) fn directory(&self, path: &str) -> io::Result<Option<Arc<[VfsMeta]>>> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let touch = tick(&mut state);
        let Some(cached) = state.directories.get_mut(&key) else {
            return Ok(None);
        };
        cached.last_touch = touch;
        cached.last_access = touch;
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
        let entry_count = entries.len().saturating_add(1);
        let byte_count = path
            .len()
            .saturating_add(meta_bytes(&metadata))
            .saturating_add(entries.iter().fold(0usize, |total, metadata| {
                total.saturating_add(meta_bytes(metadata))
            }));
        if entry_count > MAX_CACHED_ENTRIES || byte_count > MAX_CACHED_DIRECTORY_BYTES {
            return Ok(false);
        }
        let mut state = self.lock_state()?;
        let key = self.key(path);
        let root_key = self.key(&self.root);
        let previous = state.directories.get(&key).cloned();
        let last_access = previous.as_ref().map_or(0, |cached| cached.last_access);
        let available_entries = state
            .entries
            .saturating_sub(previous.as_ref().map_or(0, |cached| cached.entry_count));
        let available_bytes = state
            .bytes
            .saturating_sub(previous.as_ref().map_or(0, |cached| cached.byte_count));
        if available_entries.saturating_add(entry_count) > MAX_CACHED_ENTRIES
            || available_bytes.saturating_add(byte_count) > MAX_CACHED_BYTES
            || (previous.is_none() && state.directories.len() >= MAX_CACHED_DIRECTORIES)
        {
            evict_until(
                &mut state,
                entry_count,
                byte_count,
                &root_key,
                Some(&key),
                previous.is_none(),
            );
        }
        remove_directory(&mut state, &key);
        if !fits(&state, entry_count, byte_count)
            || state.directories.len() >= MAX_CACHED_DIRECTORIES
        {
            if let Some(previous) = previous {
                restore_directory(&mut state, key, previous);
            }
            return Ok(false);
        }
        let last_touch = tick(&mut state);
        state.generation = state.generation.saturating_add(1);
        state.entries += entry_count;
        state.bytes += byte_count;
        state.directories.insert(
            key.clone(),
            CachedDirectory {
                path: path.to_string(),
                metadata,
                entries: Arc::clone(&entries),
                depth,
                entry_count,
                byte_count,
                last_touch,
                last_access,
                refreshed_through_access: last_access,
                revision: last_touch,
            },
        );
        state.snapshot_cooldowns.remove(&key);
        reconcile_direct_children(&mut state, &key, &entries, self.case_sensitive);
        Ok(true)
    }

    pub(super) fn invalidate(&self, path: &str, recursive: bool) -> io::Result<()> {
        let mut state = self.lock_state()?;
        state.generation = state.generation.saturating_add(1);
        let key = self.key(path);
        let prefix = format!("{}/", key.trim_end_matches('/'));
        let directory_keys = state
            .directories
            .keys()
            .filter(|candidate| *candidate == &key || (recursive && candidate.starts_with(&prefix)))
            .cloned()
            .collect::<Vec<_>>();
        for candidate in directory_keys {
            remove_directory(&mut state, &candidate);
        }
        let parent_key = parent_and_name(path).map(|(parent, _)| self.key(parent));
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

    pub(super) fn refresh_targets(
        &self,
        limit: usize,
        proactive_root: bool,
    ) -> io::Result<Vec<(String, u8)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut state = self.lock_state()?;
        let now = Instant::now();
        state
            .snapshot_cooldowns
            .retain(|_, retry_at| *retry_at > now);
        let root_key = self.key(&self.root);
        let mut others = state
            .directories
            .iter()
            .filter(|(key, _)| *key != &root_key && !state.snapshot_cooldowns.contains_key(*key))
            .map(|(_, cached)| {
                (
                    cached.path.clone(),
                    cached.depth,
                    cached.last_access,
                    cached.refreshed_through_access,
                )
            })
            .collect::<Vec<_>>();
        let mut selected = Vec::with_capacity(limit);
        if !state.snapshot_cooldowns.contains_key(&root_key) {
            match state.directories.get(&root_key) {
                Some(root) => selected.push((root.path.clone(), root.depth)),
                None if proactive_root => selected.push((self.root.clone(), 0)),
                None => {}
            }
        }
        others.sort_by(|left, right| {
            let left_active = left.2 > left.3;
            let right_active = right.2 > right.3;
            right_active
                .cmp(&left_active)
                .then(right.2.cmp(&left.2))
                .then(left.0.cmp(&right.0))
        });
        let active_take = (limit - selected.len()).min(
            others
                .iter()
                .take_while(|candidate| candidate.2 > candidate.3)
                .count(),
        );
        selected.extend(
            others
                .drain(..active_take)
                .map(|(path, depth, _, _)| (path, depth)),
        );
        others.sort_by(|left, right| left.0.cmp(&right.0));
        if !others.is_empty() && selected.len() < limit {
            let take = (limit - selected.len()).min(others.len());
            let start = state.refresh_cursor % others.len();
            for offset in 0..take {
                let (path, depth, _, _) = &others[(start + offset) % others.len()];
                selected.push((path.clone(), *depth));
            }
            state.refresh_cursor = (start + take) % others.len();
        }
        Ok(selected)
    }

    pub(super) fn preload_targets(
        &self,
        maximum_depth: u8,
        limit: usize,
    ) -> io::Result<Vec<(String, u8)>> {
        if maximum_depth <= 1 || limit == 0 {
            return Ok(Vec::new());
        }
        let now = Instant::now();
        let mut state = self.lock_state()?;
        state
            .snapshot_cooldowns
            .retain(|_, retry_at| *retry_at > now);
        let mut candidates = BinaryHeap::with_capacity(limit.saturating_add(1));
        for cached in state.directories.values() {
            let child_depth = cached.depth.saturating_add(1);
            for metadata in cached.entries.iter() {
                if child_depth >= maximum_depth || !metadata.is_dir || metadata.is_symlink {
                    continue;
                }
                let path = join(&cached.path, &metadata.name);
                let key = self.key(&path);
                if state.directories.contains_key(&key)
                    || state.snapshot_cooldowns.contains_key(&key)
                {
                    continue;
                }
                let candidate = (child_depth, path);
                let replaces_largest = candidates.peek().is_some_and(|largest| {
                    candidate.0 < largest.0
                        || (candidate.0 == largest.0 && candidate.1.as_str() < largest.1.as_str())
                });
                if candidates.len() < limit {
                    candidates.push(candidate);
                } else if replaces_largest {
                    candidates.pop();
                    candidates.push(candidate);
                }
            }
        }
        let mut candidates = candidates.into_vec();
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        Ok(candidates
            .into_iter()
            .map(|(depth, path)| (path, depth))
            .collect())
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

    pub(super) fn load_guard(&self, path: &str) -> io::Result<Arc<Mutex<()>>> {
        let key = self.key(path);
        let mut loads = self
            .loads
            .lock()
            .map_err(|_| io::Error::other("metadata load table is unavailable"))?;
        loads.retain(|_, guard| guard.strong_count() > 0);
        if let Some(guard) = loads.get(&key).and_then(Weak::upgrade) {
            return Ok(guard);
        }
        let guard = Arc::new(Mutex::new(()));
        loads.insert(key, Arc::downgrade(&guard));
        Ok(guard)
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
}
