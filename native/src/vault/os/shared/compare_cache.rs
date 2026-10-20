//! The state of the comparisons this process holds (vault plan C6.1, U26-2):
//! one entry per vault job (and per checked site), in memory only and never
//! on disk. It carries the cleartext maps of both sides, so it ends with the
//! lock of the vault whose key built it (the session tells the cache) and
//! with the process. A newer comparison of the same job replaces the entry;
//! an action on an entry that is gone answers `compare_stale`.
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::bisync::{Action, Conflict};
use crate::syncjobs::SyncJob;
use crate::vault::core::compare_model::CompareItem;
use crate::vault::core::error_texts::{error_text_for_code, COMPARE_STALE_CODE};
use crate::vault::core::health_model::HealthFinding;
use crate::vault::core::{vault_locked, LockListener, VaultSession};
use crate::vfs::BackendHandle;

use super::dir_map::DirMap;
use super::sync_job::VaultPreview;

/// The cache key of a checked site (a check has no job).
pub(super) fn site_key(site: &str) -> String {
    format!("check:{site}")
}

/// One side of a comparison.
pub(super) struct SideIo {
    pub(super) backend: BackendHandle,
    pub(super) root: String,
    /// The site ID.
    pub(super) site: String,
    /// The label surfaces show.
    pub(super) label: String,
}

/// What an entry stands for.
pub(super) enum Payload {
    /// Cipher files of the comparison: their planned actions and conflicts.
    Files {
        actions: Vec<Action>,
        conflicts: Vec<Conflict>,
        /// Every cipher file of the entry (also those without an action).
        rels: Vec<String>,
        /// The file that gives the entry its sides.
        main: String,
    },
    /// A finding of the check on one side.
    Finding { side: usize, finding: HealthFinding },
}

/// An entry with what the actions need.
pub(super) struct Held {
    pub(super) item: CompareItem,
    pub(super) payload: Payload,
    /// Done: the next comparison shows the new state.
    pub(super) done: bool,
}

/// One comparison.
pub(super) struct CompareState {
    pub(super) key: String,
    pub(super) revision: u64,
    /// The site whose key built the maps.
    pub(super) site: String,
    /// The session of that key: a lock ends the use of this state.
    pub(super) session: Arc<VaultSession>,
    pub(super) job: Option<SyncJob>,
    /// Why the engine could not plan (a safety stop), shown instead of
    /// entries.
    pub(super) unavailable: Option<String>,
    pub(super) preview: Option<Mutex<VaultPreview>>,
    /// Two sides for a job, one for a checked site.
    pub(super) sides: Vec<SideIo>,
    pub(super) maps: Vec<DirMap>,
    pub(super) items: Mutex<Vec<Held>>,
    /// The entries of the groups asked for, as the comparison found them: the
    /// pages of `Items` read this list, so they stay in order while entries
    /// are being resolved.
    pub(super) listing: Arc<Vec<CompareItem>>,
    /// Which actions of the preview an entry already applied.
    pub(super) applied: Mutex<Vec<bool>>,
    /// Keeps the lock listener alive as long as the state.
    pub(super) listener: Arc<dyn LockListener>,
}

impl CompareState {
    pub(super) fn items(&self) -> MutexGuard<'_, Vec<Held>> {
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn applied(&self) -> MutexGuard<'_, Vec<bool>> {
        self.applied.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `vault_locked` once the key side is locked.
    pub(super) fn check_unlocked(&self) -> io::Result<()> {
        if self.session.is_locked() {
            Err(vault_locked(&self.site))
        } else {
            Ok(())
        }
    }
}

/// Drops the states of a site when its session locks.
pub(super) struct Dropper {
    pub(super) site: String,
}

impl LockListener for Dropper {
    fn locked(&self) {
        cache().drop_site(&self.site);
    }
}

/// The error of an action on a comparison that is gone.
pub(crate) fn stale() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        StaleComparison(error_text_for_code(COMPARE_STALE_CODE).unwrap_or("veraltet")),
    )
}

/// The payload of `stale()`; the daemon reads it as code `compare_stale`.
#[derive(Debug)]
pub(crate) struct StaleComparison(pub(crate) &'static str);

impl std::fmt::Display for StaleComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for StaleComparison {}

/// Whether `error` is the error of a comparison that is gone.
pub(crate) fn is_stale(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<StaleComparison>().is_some())
}

#[derive(Default)]
pub(super) struct CompareCache {
    states: Mutex<HashMap<String, Arc<CompareState>>>,
    revision: AtomicU64,
}

impl CompareCache {
    fn states(&self) -> MutexGuard<'_, HashMap<String, Arc<CompareState>>> {
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The number of the next comparison of this process.
    pub(super) fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Makes `state` the comparison of its key (replacing an older one) and
    /// has the session tell the cache when it locks.
    pub(super) fn put(&self, state: CompareState) -> Arc<CompareState> {
        let state = Arc::new(state);
        self.states().insert(state.key.clone(), Arc::clone(&state));
        // After the insert and outside the lock: an already locked session
        // calls the listener at once, which takes the lock again.
        state.session.watch(Arc::downgrade(&state.listener));
        state
    }

    /// The comparison of `key`; `compare_stale` when there is none,
    /// `vault_locked` when its key side locked meanwhile.
    pub(super) fn get(&self, key: &str) -> io::Result<Arc<CompareState>> {
        let state = self.states().get(key).cloned().ok_or_else(stale)?;
        if state.session.is_locked() {
            self.drop_site(&state.site);
            return Err(vault_locked(&state.site));
        }
        Ok(state)
    }

    /// Forgets every comparison built with the key of `site`.
    pub(super) fn drop_site(&self, site: &str) {
        self.states().retain(|_, state| state.site != site);
    }

    /// Whether a comparison of `key` is held.
    pub(super) fn has(&self, key: &str) -> bool {
        self.states().contains_key(key)
    }

    /// The listing of the comparison `key` of the number `revision`; a
    /// newer or older comparison is `compare_stale`.
    pub(super) fn listing(&self, key: &str, revision: u64) -> io::Result<Arc<Vec<CompareItem>>> {
        let state = self.get(key)?;
        if state.revision == revision {
            Ok(Arc::clone(&state.listing))
        } else {
            Err(stale())
        }
    }

    /// Forgets the comparison of `key`.
    pub(super) fn forget(&self, key: &str) {
        self.states().remove(key);
    }
}

/// The comparisons of this process.
pub(super) fn cache() -> &'static CompareCache {
    static CACHE: OnceLock<CompareCache> = OnceLock::new();
    CACHE.get_or_init(CompareCache::default)
}
