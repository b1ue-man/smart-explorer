use super::{order, preload, support::tick, MetadataCache};
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
        if limit == 0 { return Ok(Vec::new()); }
        let mut state = self.lock_state()?;
        let now = Instant::now();
        order::prune_cooldowns(&mut state, now);
        preload::prune_retries(&mut state, now, self.case_sensitive);
        let root_key = self.key(&self.root);
        let root_eligible = !state.snapshot_cooldowns.contains_key(&root_key);
        let root = (limit > 1 && root_eligible
            && (proactive_root || state.directories.contains_key(&root_key)))
            .then(|| root_key.clone());
        // Three persistent indexes visit only selected/skipped batch positions:
        // one oldest attempt, one latest demand, then outstanding demand/recent
        // access. Moving failed attempts to the tail preserves cold fairness.
        let mut selected = state.refresh.select(root, limit);
        if selected.is_empty() && proactive_root && root_eligible { selected.push(root_key); }
        let attempt = tick(&mut state);
        Ok(selected.into_iter().filter_map(|key| {
            if let Some(cached) = state.directories.get_mut(&key) {
                cached.last_attempt = attempt;
                Some((cached.path.clone(), cached.depth, Some(cached.revision)))
            } else if key == self.key(&self.root) {
                Some((self.root.clone(), 0, None))
            } else { None }
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
}
