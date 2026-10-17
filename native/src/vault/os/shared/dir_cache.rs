//! Directory cache of one vault backend (R-10: fewer round trips on slow
//! storage): cleartext folder → directory ID and content directory, and per
//! folder the display names of foreign conflict copies (`AliasMap`).
//!
//! Keys are cleartext paths, so the cache lives only in memory and is
//! dropped when the vault locks (K46); after that nothing new is stored.
//! Entries expire after 20 s like the browsing cache (`vfs::CachingBackend`),
//! because other clients change the vault too; this backend's own changes
//! invalidate what they touch.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::vault::core::listing::AliasMap;
use crate::vault::core::path_map::key_within;
use crate::vault::core::LockListener;

/// Lifetime of an entry.
const TTL: Duration = Duration::from_secs(20);
/// Folders kept at most; beyond that expired entries go, then all.
const MAX_FOLDERS: usize = 4096;

/// A folder of the vault as stored: its directory ID and its content
/// directory (`d/<2>/<30>`, relative to the vault root).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DirRef {
    pub(super) id: String,
    pub(super) content: String,
}

#[derive(Default)]
struct State {
    dirs: HashMap<String, (DirRef, Instant)>,
    aliases: HashMap<String, (Arc<AliasMap>, Instant)>,
}

#[derive(Default)]
pub(super) struct DirCache {
    state: Mutex<State>,
    locked: AtomicBool,
}

impl DirCache {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn dir(&self, key: &str) -> Option<DirRef> {
        let state = self.state();
        let (dir, at) = state.dirs.get(key)?;
        (at.elapsed() < TTL).then(|| dir.clone())
    }

    pub(super) fn put_dir(&self, key: &str, dir: &DirRef) {
        // Checked under the mutex: `locked` sets the flag before it clears.
        let mut state = self.state();
        if self.locked.load(Ordering::SeqCst) {
            return;
        }
        if state.dirs.len() >= MAX_FOLDERS {
            state.dirs.retain(|_, (_, at)| at.elapsed() < TTL);
            if state.dirs.len() >= MAX_FOLDERS {
                state.dirs.clear();
            }
        }
        state
            .dirs
            .insert(key.to_string(), (dir.clone(), Instant::now()));
    }

    pub(super) fn aliases(&self, key: &str) -> Option<Arc<AliasMap>> {
        let state = self.state();
        let (aliases, at) = state.aliases.get(key)?;
        (at.elapsed() < TTL).then(|| Arc::clone(aliases))
    }

    pub(super) fn put_aliases(&self, key: &str, aliases: AliasMap) -> Arc<AliasMap> {
        let aliases = Arc::new(aliases);
        let mut state = self.state();
        if self.locked.load(Ordering::SeqCst) {
            return aliases;
        }
        if state.aliases.len() >= MAX_FOLDERS {
            state.aliases.retain(|_, (_, at)| at.elapsed() < TTL);
            if state.aliases.len() >= MAX_FOLDERS {
                state.aliases.clear();
            }
        }
        state
            .aliases
            .insert(key.to_string(), (Arc::clone(&aliases), Instant::now()));
        aliases
    }

    /// Forgets the folder `key` and everything below it (rename, removal).
    pub(super) fn forget_tree(&self, key: &str) {
        let mut state = self.state();
        state.dirs.retain(|cached, _| !key_within(cached, key));
        state.aliases.retain(|cached, _| !key_within(cached, key));
    }

    /// Forgets the display names of the folder `key` (an entry came or went).
    pub(super) fn forget_aliases(&self, key: &str) {
        self.state().aliases.remove(key);
    }

    /// Forgets everything (explicit refresh).
    pub(super) fn clear(&self) {
        let mut state = self.state();
        state.dirs.clear();
        state.aliases.clear();
    }

    pub(super) fn is_empty(&self) -> bool {
        let state = self.state();
        state.dirs.is_empty() && state.aliases.is_empty()
    }
}

impl LockListener for DirCache {
    fn locked(&self) {
        self.locked.store(true, Ordering::SeqCst);
        let mut state = self.state();
        state.dirs.clear();
        state.aliases.clear();
    }
}
