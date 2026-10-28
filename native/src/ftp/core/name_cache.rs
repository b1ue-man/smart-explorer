//! Folder names a transfer job already listed (G3-04b, 2026-10-28). FTP has
//! no no-replace rename: a publication proves the destination absent first,
//! and for a missing name only a complete listing of its folder proves that
//! (a 550 may also mean "denied"). Listing the folder once per published file
//! costs O(n²) listing bytes for n files in one folder.
//!
//! While a job holds a `ListingScope`, a folder listed for a publication is
//! kept with every name in it and updated by this backend's own writes
//! (published names added, the folder dropped after any other change this
//! backend makes in it and after a refused RNTO). There is no time-based
//! refresh: names that appear from elsewhere during the job are caught by the
//! exact probe of the destination name right before RNFR/RNTO (`MLST`), and a
//! foreign folder under the name makes RNTO fail. When the last scope ends,
//! everything is forgotten; without a scope nothing is kept and every
//! publication lists as before.
use super::core_impl::{basename, parent_dir};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct CacheState {
    scopes: usize,
    folders: HashMap<String, HashSet<String>>,
}

#[derive(Default)]
pub(super) struct NameCache {
    state: Mutex<CacheState>,
}

fn lock(state: &Mutex<CacheState>) -> MutexGuard<'_, CacheState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps the listings of one transfer job (`FtpBackend::listing_scope`).
pub struct ListingScope(Arc<NameCache>);

impl Drop for ListingScope {
    fn drop(&mut self) {
        let mut state = lock(&self.0.state);
        state.scopes = state.scopes.saturating_sub(1);
        if state.scopes == 0 {
            state.folders.clear();
        }
    }
}

impl NameCache {
    pub(super) fn begin(self: &Arc<Self>) -> ListingScope {
        lock(&self.state).scopes += 1;
        ListingScope(self.clone())
    }

    /// Whether a job keeps listings now.
    pub(super) fn active(&self) -> bool {
        lock(&self.state).scopes > 0
    }

    /// Whether `path`'s name is in its folder's kept listing; `None` when
    /// the folder is not kept (no scope, or not listed yet).
    pub(super) fn contains(&self, path: &str) -> Option<bool> {
        let state = lock(&self.state);
        state
            .folders
            .get(&parent_dir(path))
            .map(|names| names.contains(&basename(path)))
    }

    /// Keeps a complete listing of `folder` while a scope is open.
    pub(super) fn fill(&self, folder: &str, names: impl IntoIterator<Item = String>) {
        let mut state = lock(&self.state);
        if state.scopes > 0 {
            state
                .folders
                .insert(folder.to_string(), names.into_iter().collect());
        }
    }

    /// This backend published (or created) `path`.
    pub(super) fn added(&self, path: &str) {
        let mut state = lock(&self.state);
        if let Some(names) = state.folders.get_mut(&parent_dir(path)) {
            names.insert(basename(path));
        }
    }

    /// Any other change this backend made next to `path`, or an answer that
    /// contradicts the kept listing: the folder is listed again when needed.
    pub(super) fn changed(&self, path: &str) {
        lock(&self.state).folders.remove(&parent_dir(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_ftp_name_cache_lives_only_within_a_scope() {
        let cache = Arc::new(NameCache::default());
        cache.fill("/up", vec!["a".to_string()]);
        assert_eq!(cache.contains("/up/a"), None, "no scope: nothing kept");
        let scope = cache.begin();
        cache.fill("/up", vec!["a".to_string()]);
        assert_eq!(cache.contains("/up/a"), Some(true));
        assert_eq!(cache.contains("/up/b"), Some(false));
        cache.added("/up/b");
        assert_eq!(cache.contains("/up/b"), Some(true), "own publication");
        cache.changed("/up/c");
        assert_eq!(cache.contains("/up/a"), None, "own change: listed again");
        cache.fill("/up", vec!["a".to_string()]);
        drop(scope);
        assert_eq!(cache.contains("/up/a"), None, "the job ended");
    }
}
