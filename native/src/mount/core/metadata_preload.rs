//! Bounded discovery and selected-work ownership under the snapshot-state lock.
use super::{order, support::{join, parent_and_name}, CacheState, MetadataCache, MAX_CACHED_BYTES};
use super::preload_records::ChildKey;
use std::io;
use std::time::Instant;

const DISCOVERY_BUDGET: usize = 4096;

#[cfg(test)]
#[path = "metadata_preload_task_tests.rs"]
mod task_tests;

pub(in crate::mount) struct PreloadTicket {
    pub(in crate::mount) path: String,
    pub(in crate::mount) depth: u8,
    key: ChildKey,
    target_key: String,
}

pub(in crate::mount) struct PreloadBatch<'a> {
    cache: &'a MetadataCache,
    tickets: Vec<PreloadTicket>,
    maximum_depth: u8,
}

impl PreloadBatch<'_> {
    pub(in crate::mount) fn tickets(&self) -> &[PreloadTicket] { &self.tickets }

    pub(in crate::mount) fn finish(mut self) -> io::Result<bool> {
        let mut state = self.cache.lock_state()?;
        for ticket in &self.tickets { finish_ticket(&mut state, ticket); }
        self.tickets.clear();
        Ok(has_work(&state, self.maximum_depth))
    }
}

impl Drop for PreloadBatch<'_> {
    fn drop(&mut self) {
        if self.tickets.is_empty() { return; }
        if let Ok(mut state) = self.cache.lock_state() {
            // Includes targets canceled before dispatch and errors/panics after
            // dispatch. Old revisions cannot restore work into a newer parent.
            for ticket in &self.tickets { finish_ticket(&mut state, ticket); }
        }
    }
}

fn has_work(state: &CacheState, maximum_depth: u8) -> bool {
    state.bytes.saturating_add(state.cooldown_bytes) < MAX_CACHED_BYTES
        && state.preload.has_work(maximum_depth)
}

pub(super) fn ticket_current(state: &CacheState, ticket: &PreloadTicket) -> bool {
    state.preload.selected(&ticket.key)
        && state.directories.get(ticket.key.parent.as_ref()).is_some_and(|parent| {
            parent.revision == ticket.key.revision && !parent.comparison_only
                && parent.listing_expires_at > Instant::now()
        })
}

fn finish_ticket(state: &mut CacheState, ticket: &PreloadTicket) {
    if !state.preload.selected(&ticket.key) { return; }
    state.preload.cover(ticket.key.parent.as_ref(), ticket.key.index);
    rearm_child(state, &ticket.target_key);
}

/// Exact parent/name lookup: no tree scan and no duplicate retry tickets.
pub(super) fn rearm_child(state: &mut CacheState, key: &str) {
    let Some((parent_key, name)) = parent_and_name(key) else { return; };
    let Some(parent) = state.directories.get(parent_key) else { return; };
    let Some(index) = parent.entry_index.get(name).copied() else { return; };
    let Some(metadata) = parent.entries.get(index) else { return; };
    if !metadata.is_dir || metadata.is_symlink { return; }
    if !parent.comparison_only && parent.listing_expires_at > Instant::now()
        && !state.directories.contains_key(key) && !state.snapshot_cooldowns.contains_key(key)
    {
        state.preload.rearm(parent_key, index);
    } else {
        state.preload.cover(parent_key, index);
    }
}

pub(super) fn clear_retry(state: &mut CacheState, key: &str) {
    let Some((parent, name)) = parent_and_name(key) else { return; };
    if let Some(index) = state.directories.get(parent)
        .and_then(|cached| cached.entry_index.get(name)).copied()
    {
        state.preload.cover(parent, index);
    }
}

pub(super) fn cool_down_child(state: &mut CacheState, key: &str, deadline: Instant) {
    let Some((parent, name)) = parent_and_name(key) else { return; };
    if let Some(index) = state.directories.get(parent)
        .and_then(|cached| cached.entry_index.get(name)).copied()
    {
        state.preload.cool_down(parent, index, deadline);
    }
}

pub(super) fn prune_retries(state: &mut CacheState, now: Instant, case_sensitive: bool) {
    while let Some(child) = state.preload.retry_due(now) {
        let key = state.directories.get(child.parent.as_ref())
            .filter(|parent| parent.revision == child.revision)
            .and_then(|parent| parent.entries.get(child.index))
            .map(|entry| join(child.parent.as_ref(), &super::identity_key(case_sensitive, &entry.name)));
        if let Some(key) = key { rearm_child(state, &key); }
    }
}

impl MetadataCache {
    pub(in crate::mount) fn preload_ticket_current(&self, ticket: &PreloadTicket) -> io::Result<bool> {
        let state = self.lock_state()?;
        Ok(ticket_current(&state, ticket))
    }

    pub(in crate::mount) fn select_preload(
        &self, maximum_depth: u8, limit: usize,
    ) -> io::Result<PreloadBatch<'_>> {
        let mut batch = PreloadBatch { cache: self, tickets: Vec::new(), maximum_depth };
        if maximum_depth <= 1 || limit == 0 { return Ok(batch); }
        let mut state = self.lock_state()?;
        let now = Instant::now();
        order::prune_cooldowns(&mut state, now);
        prune_retries(&mut state, now, self.case_sensitive);
        if !has_work(&state, maximum_depth) { return Ok(batch); }
        let mut inspected = 0;
        while batch.tickets.len() < limit && inspected < DISCOVERY_BUDGET {
            let ready = state.preload.ready(maximum_depth);
            let cursor = state.preload.cursor(maximum_depth);
            if let Some((key, depth)) = ready.filter(|(_, depth)| {
                cursor.as_ref().map_or(true, |(_, _, cursor_depth)| depth <= cursor_depth)
            }) {
                inspected += 1;
                state.preload.select(&key);
                let path = state.directories.get(key.parent.as_ref()).and_then(|parent| {
                    (parent.revision == key.revision && !parent.comparison_only
                        && parent.listing_expires_at > Instant::now())
                        .then(|| parent.entries.get(key.index).map(|entry| join(&parent.path, &entry.name)))
                        .flatten()
                });
                if let Some(path) = path {
                    let child_key = self.key(&path);
                    if !state.directories.contains_key(&child_key)
                        && !state.snapshot_cooldowns.contains_key(&child_key)
                    {
                        batch.tickets.push(PreloadTicket { path, depth, key, target_key: child_key });
                        continue;
                    }
                }
                state.preload.cover(key.parent.as_ref(), key.index);
                continue;
            }
            let Some((parent_key, index, _)) = cursor else { break; };
            inspected += 1;
            let Some(parent) = state.directories.get(parent_key.as_ref()) else {
                state.preload.remove(parent_key.as_ref());
                continue;
            };
            if parent.comparison_only || parent.listing_expires_at <= Instant::now()
                || index >= parent.entries.len()
            {
                state.preload.advance(parent_key.as_ref(), index, true);
                continue;
            }
            let entry = &parent.entries[index];
            let child_key = (entry.is_dir && !entry.is_symlink)
                .then(|| self.key(&join(&parent.path, &entry.name)));
            let finished = index + 1 == parent.entries.len();
            state.preload.advance(parent_key.as_ref(), index + 1, finished);
            if let Some(key) = child_key {
                let eligible = !state.directories.contains_key(&key)
                    && !state.snapshot_cooldowns.contains_key(&key);
                state.preload.discover(parent_key.as_ref(), index, eligible);
            }
        }
        Ok(batch)
    }

    /// Compatibility inspection releases its selections immediately; production
    /// holds a PreloadBatch until every dispatched worker has joined.
    pub(in crate::mount) fn preload_targets(
        &self, maximum_depth: u8, limit: usize,
    ) -> io::Result<Vec<(String, u8)>> {
        let batch = self.select_preload(maximum_depth, limit)?;
        Ok(batch.tickets.iter().map(|ticket| (ticket.path.clone(), ticket.depth)).collect())
    }
}
