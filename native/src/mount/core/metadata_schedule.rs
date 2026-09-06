use super::{order, support::*, MetadataCache, MAX_CACHED_BYTES};
use std::collections::BinaryHeap;
use std::io;
use std::time::Instant;

impl MetadataCache {
    pub(in crate::mount) fn refresh_targets(
        &self, limit: usize, proactive_root: bool,
    ) -> io::Result<Vec<(String, u8)>> {
        self.refresh_targets_with_revisions(limit, proactive_root)
            .map(|targets| targets.into_iter().map(|(path, depth, _)| (path, depth)).collect())
    }

    pub(in crate::mount) fn refresh_targets_with_revisions(
        &self, limit: usize, proactive_root: bool,
    ) -> io::Result<Vec<(String, u8, Option<u64>)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut state = self.lock_state()?;
        let now = Instant::now();
        order::prune_cooldowns(&mut state, now);
        let root_key = self.key(&self.root);
        let mut candidates = state.directories.iter()
            .filter(|(key, _)| !state.snapshot_cooldowns.contains_key(*key))
            .map(|(_, cached)| (cached.path.clone(), cached.depth, cached.last_access,
                cached.refreshed_through_access, cached.last_attempt))
            .collect::<Vec<_>>();
        let mut selected = Vec::with_capacity(limit.min(candidates.len() + 1));
        if limit > 1 && !state.snapshot_cooldowns.contains_key(&root_key) {
            if let Some(index) = candidates.iter().position(|candidate| candidate.0 == self.root) {
                let root = candidates.remove(index);
                selected.push((root.0, root.1));
            } else if proactive_root {
                selected.push((self.root.clone(), 0));
            }
        }
        // Reserve one position for the least recently attempted snapshot. A
        // failed refresh advances its attempt clock too, so it cannot starve
        // another cold directory by remaining the oldest successful refresh.
        candidates.sort_by(|left, right| left.4.cmp(&right.4).then(left.0.cmp(&right.0)));
        if selected.len() < limit && !candidates.is_empty() {
            let oldest = candidates.remove(0);
            selected.push((oldest.0, oldest.1));
        }
        // An application may keep watching its latest directory without reading
        // it again. Servicing its last demand must not put it behind an entire
        // earlier vault scan. Keep one recent position as well as cold fairness.
        if selected.len() < limit {
            if let Some(index) = candidates.iter().enumerate()
                .filter(|(_, candidate)| candidate.2 > 0)
                .max_by_key(|(_, candidate)| candidate.2).map(|(index, _)| index)
            {
                let recent = candidates.remove(index);
                selected.push((recent.0, recent.1));
            }
        }
        candidates.sort_by(|left, right| (right.2 > right.3).cmp(&(left.2 > left.3))
            .then(right.2.cmp(&left.2)).then(left.0.cmp(&right.0)));
        let remaining = limit - selected.len();
        selected.extend(candidates.into_iter().take(remaining)
            .map(|(path, depth, _, _, _)| (path, depth)));
        if selected.is_empty() && proactive_root
            && !state.snapshot_cooldowns.contains_key(&root_key)
        {
            selected.push((self.root.clone(), 0));
        }
        let attempt = tick(&mut state);
        for (path, _) in &selected {
            if let Some(cached) = state.directories.get_mut(&self.key(path)) {
                cached.last_attempt = attempt;
            }
        }
        // Capture identity under the same lock as selection. A foreground load
        // that completes while this target waits can satisfy that maintenance.
        Ok(selected.into_iter().map(|(path, depth)| {
            let revision = state.directories.get(&self.key(&path)).map(|cached| cached.revision);
            (path, depth, revision)
        }).collect())
    }

    pub(in crate::mount) fn refreshed_since(
        &self, path: &str, selected_revision: Option<u64>,
    ) -> io::Result<bool> {
        let state = self.lock_state()?;
        Ok(state.directories.get(&self.key(path)).is_some_and(|cached| {
            Some(cached.revision) != selected_revision && cached.listing_expires_at > Instant::now()
        }))
    }

    pub(in crate::mount) fn preload_targets(
        &self, maximum_depth: u8, limit: usize,
    ) -> io::Result<Vec<(String, u8)>> {
        if maximum_depth <= 1 || limit == 0 {
            return Ok(Vec::new());
        }
        let now = Instant::now();
        let mut state = self.lock_state()?;
        order::prune_cooldowns(&mut state, now);
        if state.bytes.saturating_add(state.cooldown_bytes) >= MAX_CACHED_BYTES {
            return Ok(Vec::new());
        }
        let mut candidates: BinaryHeap<(u8, String)> = BinaryHeap::with_capacity(limit + 1);
        for cached in state.directories.values() {
            if cached.listing_expires_at <= now {
                continue;
            }
            let child_depth = cached.depth.saturating_add(1);
            for metadata in cached.entries.iter() {
                if child_depth >= maximum_depth || !metadata.is_dir || metadata.is_symlink {
                    continue;
                }
                let path = join(&cached.path, &metadata.name);
                let key = self.key(&path);
                if state.directories.contains_key(&key) || state.snapshot_cooldowns.contains_key(&key) {
                    continue;
                }
                let candidate = (child_depth, path);
                let replaces_largest = candidates.peek().is_some_and(|largest| candidate < *largest);
                if candidates.len() < limit {
                    candidates.push(candidate);
                } else if replaces_largest {
                    candidates.pop();
                    candidates.push(candidate);
                }
            }
        }
        let mut candidates = candidates.into_vec();
        candidates.sort();
        Ok(candidates.into_iter().map(|(depth, path)| (path, depth)).collect())
    }
}
