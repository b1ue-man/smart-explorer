//! Point-cache path, recency and expiry indexes.
use super::{CachedPoint, PointState};
use std::collections::BTreeMap;
use std::time::Instant;

pub(super) fn descendants<T>(map: &BTreeMap<String, T>, key: &str) -> Vec<String> {
    let prefix = format!("{}/", key.trim_end_matches('/'));
    map.range(prefix.clone()..).take_while(|(path, _)| path.starts_with(&prefix))
        .filter(|(path, _)| path.as_str() != key)
        .map(|(path, _)| path.clone()).collect()
}

pub(super) fn remove(state: &mut PointState, key: &str) {
    if let Some(cached) = state.entries.remove(key) {
        state.recency.remove(&(cached.last_touch, key.to_string()));
        state.expiry.remove(&(cached.expires_at, key.to_string()));
        state.bytes = state.bytes.saturating_sub(cached.bytes);
    }
}

pub(super) fn prune_expired(state: &mut PointState) {
    let now = Instant::now();
    while let Some((deadline, key)) = state.expiry.first() {
        if *deadline > now { break; }
        let key = key.clone();
        remove(state, &key);
    }
}

pub(super) fn touch(state: &mut PointState, key: &str) {
    state.clock = state.clock.saturating_add(1);
    if let Some(cached) = state.entries.get_mut(key) {
        state.recency.remove(&(cached.last_touch, key.to_string()));
        cached.last_touch = state.clock;
        state.recency.insert((cached.last_touch, key.to_string()));
    }
}

pub(super) fn insert(state: &mut PointState, key: String, cached: CachedPoint) {
    state.bytes = state.bytes.saturating_add(cached.bytes);
    state.recency.insert((cached.last_touch, key.clone()));
    state.expiry.insert((cached.expires_at, key.clone()));
    state.entries.insert(key, cached);
}
