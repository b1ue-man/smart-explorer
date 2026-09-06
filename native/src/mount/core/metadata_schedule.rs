use super::{support::*, MetadataCache, MAX_CACHED_BYTES, MAX_CACHED_DIRECTORIES,
    MAX_CACHED_ENTRIES};
use std::collections::BinaryHeap;
use std::io;
use std::time::Instant;

impl MetadataCache {
    pub(in crate::mount) fn refresh_targets(
        &self, limit: usize, proactive_root: bool,
    ) -> io::Result<Vec<(String, u8)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut state = self.lock_state()?;
        let now = Instant::now();
        state.snapshot_cooldowns.retain(|_, retry_at| *retry_at > now);
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
        Ok(selected)
    }

    pub(in crate::mount) fn preload_targets(
        &self, maximum_depth: u8, limit: usize,
    ) -> io::Result<Vec<(String, u8)>> {
        if maximum_depth <= 1 || limit == 0 {
            return Ok(Vec::new());
        }
        let now = Instant::now();
        let mut state = self.lock_state()?;
        state.snapshot_cooldowns.retain(|_, retry_at| *retry_at > now);
        if state.directories.len() >= MAX_CACHED_DIRECTORIES
            || state.entries >= MAX_CACHED_ENTRIES || state.bytes >= MAX_CACHED_BYTES
        {
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

/// Ancestor waves commit before descendants begin; every started worker is
/// joined, including after spawn failure or another worker's panic/error.
pub(in crate::mount) fn run_metadata_batch(
    mut targets: Vec<(String, u8)>, width: usize,
    stopped: &(impl Fn() -> bool + Sync),
    work: &(impl Fn(&str, u8) -> io::Result<bool> + Sync),
) -> io::Result<usize> {
    targets.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
    let mut completed = 0;
    let mut first_error = None;
    let mut start = 0;
    while start < targets.len() {
        let depth = targets[start].1;
        let end = start + targets[start..].iter().take_while(|(_, level)| *level == depth).count();
        for batch in targets[start..end].chunks(width.clamp(1, 4)) {
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(batch.len());
                for (path, depth) in batch {
                    match std::thread::Builder::new().name("mount-metadata-load".into())
                        .spawn_scoped(scope, move || {
                            if stopped() { Ok(false) } else { work(path, *depth) }
                        })
                    {
                        Ok(handle) => handles.push(handle),
                        Err(error) => { first_error.get_or_insert(error); break; }
                    }
                }
                for handle in handles {
                    let result = handle.join().unwrap_or_else(|_| {
                        Err(io::Error::other("mounted metadata worker panicked"))
                    });
                    collect(result, &mut completed, &mut first_error);
                }
            });
        }
        start = end;
    }
    first_error.map_or(Ok(completed), Err)
}

fn collect(result: io::Result<bool>, completed: &mut usize, error: &mut Option<io::Error>) {
    match result {
        Ok(true) => *completed += 1,
        Ok(false) => {}
        Err(failure) => { error.get_or_insert(failure); }
    }
}
