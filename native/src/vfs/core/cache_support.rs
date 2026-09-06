use std::mem::size_of;
use std::sync::Mutex;

use crate::vfs::VfsMeta;

use super::{CacheLimits, CacheState, CachedDirectory, CachingBackend};
use super::cache_load::DirectorySnapshot;
use std::time::Instant;

pub(super) fn tick(cache: &mut CacheState) -> u64 {
    cache.clock = cache.clock.saturating_add(1);
    cache.clock
}

pub(super) fn remove_directory(cache: &mut CacheState, key: &str) {
    if let Some(previous) = cache.directories.remove(key) {
        cache.recency.remove(&(previous.last_touch, key.to_string()));
        cache.expiry.remove(&(previous.snapshot.expires_at, key.to_string()));
        cache.entries = cache.entries.saturating_sub(previous.entry_count);
        cache.bytes = cache.bytes.saturating_sub(previous.byte_count);
    }
}

pub(super) fn purge_expired(cache: &mut CacheState) {
    let now = Instant::now();
    while let Some((expires, key)) = cache.expiry.first().cloned() {
        if expires > now { break; }
        remove_directory(cache, &key);
    }
}

pub(super) fn cached_snapshot(cache: &mut CacheState, key: &str) -> Option<DirectorySnapshot> {
    let cached = cache.directories.get(key)?;
    if cached.snapshot.expires_at <= Instant::now() {
        remove_directory(cache, key);
        return None;
    }
    let previous_touch = cached.last_touch;
    cache.recency.remove(&(previous_touch, key.to_string()));
    let touch = tick(cache);
    let cached = cache.directories.get_mut(key)?;
    cached.last_touch = touch;
    let snapshot = cached.snapshot.clone();
    cache.recency.insert((touch, key.to_string()));
    Some(snapshot)
}

pub(super) fn fits(cache: &CacheState, entries: usize, bytes: usize, limits: CacheLimits) -> bool {
    cache.entries.saturating_add(entries) <= limits.entries
        && cache.bytes.saturating_add(bytes) <= limits.bytes
}

pub(super) fn evict_until(cache: &mut CacheState, entries: usize, bytes: usize, limits: CacheLimits) {
    while cache.directories.len() >= limits.directories || !fits(cache, entries, bytes, limits) {
        let Some((_, victim)) = cache.recency.first().cloned() else {
            break;
        };
        remove_directory(cache, &victim);
    }
}

pub(super) fn cached_metadata_bytes(key: &str, entries: &[VfsMeta]) -> usize {
    size_of::<CachedDirectory>()
        // Directory map plus maintained recency/expiry keys and tree-node
        // overhead are charged as well as the immutable metadata/index.
        .saturating_add(key.len().saturating_mul(3))
        .saturating_add(3 * (size_of::<String>() + 96))
        .saturating_add(entries.iter().fold(0usize, |total, metadata| {
            total
                .saturating_add(size_of::<VfsMeta>())
                .saturating_add(metadata.name.capacity())
                .saturating_add(metadata.id.as_ref().map_or(0, String::capacity))
                .saturating_add(metadata.content_md5.as_ref().map_or(0, String::capacity))
        }))
}

pub(super) fn cached_bytes(metadata_bytes: usize, index_bytes: usize) -> usize {
    metadata_bytes.saturating_add(index_bytes)
}

pub(super) fn invalidate_shared(cache: &Mutex<CacheState>, path: &str) {
    if let Ok(mut cache) = cache.lock() {
        cache.generation = cache.generation.wrapping_add(1);
        let key = CachingBackend::norm(path);
        remove_directory(&mut cache, &key);
        if let Some(parent) = CachingBackend::parent_of(&key) {
            remove_directory(&mut cache, &parent);
        }
    }
}

pub(super) fn invalidate_ancestors(cache: &Mutex<CacheState>, path: &str) {
    if let Ok(mut cache) = cache.lock() {
        cache.generation = cache.generation.wrapping_add(1);
        let mut current = CachingBackend::norm(path);
        loop {
            remove_directory(&mut cache, &current);
            if current == "/" {
                break;
            }
            let Some(parent) = CachingBackend::parent_of(&current) else {
                break;
            };
            current = parent;
        }
    }
}
