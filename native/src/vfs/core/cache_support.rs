use std::mem::size_of;
use std::sync::Mutex;

use crate::vfs::VfsMeta;

use super::{
    CacheState, CachedDirectory, CachingBackend, MAX_CACHED_BYTES, MAX_CACHED_DIRECTORIES,
    MAX_CACHED_ENTRIES,
};

pub(super) fn tick(cache: &mut CacheState) -> u64 {
    cache.clock = cache.clock.saturating_add(1);
    cache.clock
}

pub(super) fn remove_directory(cache: &mut CacheState, key: &str) {
    if let Some(previous) = cache.directories.remove(key) {
        cache.entries = cache.entries.saturating_sub(previous.entry_count);
        cache.bytes = cache.bytes.saturating_sub(previous.byte_count);
    }
}

pub(super) fn purge_expired(cache: &mut CacheState) {
    let expired = cache
        .directories
        .iter()
        .filter(|(_, cached)| cached.stored_at.elapsed() >= super::CACHE_TTL)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in expired {
        remove_directory(cache, &key);
    }
}

pub(super) fn fits(cache: &CacheState, entries: usize, bytes: usize) -> bool {
    cache.entries.saturating_add(entries) <= MAX_CACHED_ENTRIES
        && cache.bytes.saturating_add(bytes) <= MAX_CACHED_BYTES
}

pub(super) fn evict_until(cache: &mut CacheState, entries: usize, bytes: usize) {
    while cache.directories.len() >= MAX_CACHED_DIRECTORIES || !fits(cache, entries, bytes) {
        let victim = cache
            .directories
            .iter()
            .min_by_key(|(_, cached)| cached.last_touch)
            .map(|(key, _)| key.clone());
        let Some(victim) = victim else {
            break;
        };
        remove_directory(cache, &victim);
    }
}

pub(super) fn cached_metadata_bytes(key: &str, entries: &[VfsMeta]) -> usize {
    size_of::<CachedDirectory>()
        .saturating_add(key.len())
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
