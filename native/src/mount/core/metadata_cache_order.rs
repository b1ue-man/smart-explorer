//! Ordered retention and affected-prefix operations; never performs I/O.
use super::{CacheState, CachedDirectory};
use std::collections::BTreeMap;
use std::time::Instant;

pub(super) fn descendants<T>(map: &BTreeMap<String, T>, key: &str) -> Vec<String> {
    let prefix = format!("{}/", key.trim_end_matches('/'));
    map.range(prefix.clone()..).take_while(|(path, _)| path.starts_with(&prefix))
        .map(|(path, _)| path.clone()).collect()
}

pub(super) fn touch(state: &mut CacheState, key: &str, access: bool) {
    state.clock = state.clock.saturating_add(1);
    let now = state.clock;
    if let Some(cached) = state.directories.get_mut(key) {
        state.recency.remove(&(cached.last_touch, key.to_string()));
        cached.last_touch = now;
        if access { cached.last_access = now; }
        if !cached.deferred_changes {
            state.recency.insert((now, key.to_string()));
        }
    }
}

pub(super) fn pin_changes(state: &mut CacheState, key: &str) {
    if let Some(cached) = state.directories.get_mut(key) {
        cached.deferred_changes = true;
        state.recency.remove(&(cached.last_touch, key.to_string()));
    }
}

pub(super) fn remove(state: &mut CacheState, key: &str) -> Option<CachedDirectory> {
    let previous = state.directories.remove(key)?;
    state.recency.remove(&(previous.last_touch, key.to_string()));
    state.expiry.remove(&(previous.listing_expires_at, key.to_string()));
    state.entries = state.entries.saturating_sub(previous.entry_count);
    state.bytes = state.bytes.saturating_sub(previous.byte_count);
    Some(previous)
}

pub(super) fn insert(state: &mut CacheState, key: String, cached: CachedDirectory) {
    state.entries = state.entries.saturating_add(cached.entry_count);
    state.bytes = state.bytes.saturating_add(cached.byte_count);
    if !cached.deferred_changes {
        state.recency.insert((cached.last_touch, key.clone()));
    }
    state.expiry.insert((cached.listing_expires_at, key.clone()));
    state.directories.insert(key, cached);
}

pub(super) fn expire(state: &mut CacheState, key: &str, now: Instant) {
    if let Some(cached) = state.directories.get_mut(key) {
        state.expiry.remove(&(cached.listing_expires_at, key.to_string()));
        cached.listing_expires_at = now;
        cached.metadata_expires_at = now;
        state.expiry.insert((now, key.to_string()));
    }
}

pub(super) fn remove_cooldown(state: &mut CacheState, key: &str) {
    if let Some(deadline) = state.snapshot_cooldowns.remove(key) {
        state.cooldown_expiry.remove(&(deadline, key.to_string()));
        state.cooldown_bytes = state.cooldown_bytes.saturating_sub(cooldown_bytes(key));
    }
}

pub(super) fn cooldown_bytes(key: &str) -> usize {
    key.len().saturating_mul(2).saturating_add(128)
}

pub(super) fn prune_cooldowns(state: &mut CacheState, now: Instant) {
    while let Some((deadline, _)) = state.cooldown_expiry.first() {
        if *deadline > now { break; }
        if let Some((_, key)) = state.cooldown_expiry.pop_first() {
            state.snapshot_cooldowns.remove(&key);
            state.cooldown_bytes = state.cooldown_bytes.saturating_sub(cooldown_bytes(&key));
        }
    }
}
