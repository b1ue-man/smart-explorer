//! Path-cache persistence off the request path (plan C2): changes only mark
//! the cache dirty, one background writer saves it at most every few seconds,
//! and serialization runs on a snapshot outside the map locks. Folder-journal
//! steps that need the mapping on disk first still wait for a write, shared
//! with every caller that waits at the same time. The last backend clone
//! writes whatever is still pending.
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

type Map = Arc<Mutex<HashMap<String, String>>>;

/// Background writes are at least this far apart: a crash loses at most a few
/// seconds of cache progress, which only repeats some lookups later.
const MIN_INTERVAL: Duration = Duration::from_secs(2);
/// A background write may take at most a tenth of the time between writes, so
/// even a cache of hundreds of thousands of paths keeps its disk and CPU share
/// bounded during a large scan.
const COST_FACTOR: u32 = 10;

pub(super) struct CacheStore {
    shared: Arc<Shared>,
}

struct Shared {
    /// `None` keeps the cache in memory only (unit tests).
    path: Option<PathBuf>,
    ids: Map,
    mimes: Map,
    interval: Duration,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// Generation of the latest change.
    dirty: u64,
    /// Generation contained in the last successful write.
    written: u64,
    writing: bool,
    worker: bool,
    closing: bool,
    last_cost: Duration,
    writes: u64,
}

impl CacheStore {
    pub(super) fn new(path: Option<PathBuf>, ids: Map, mimes: Map) -> Self {
        Self::with_interval(path, ids, mimes, MIN_INTERVAL)
    }

    pub(super) fn with_interval(
        path: Option<PathBuf>,
        ids: Map,
        mimes: Map,
        interval: Duration,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                path,
                ids,
                mimes,
                interval,
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
            }),
        }
    }

    /// The maps changed; the background writer saves them soon.
    pub(super) fn mark_dirty(&self) {
        let shared = &self.shared;
        if shared.path.is_none() {
            return;
        }
        let mut state = shared.lock();
        state.dirty += 1;
        if !state.worker && !state.closing {
            let worker = Arc::clone(shared);
            let spawned = std::thread::Builder::new()
                .name("gdrive-path-cache".into())
                .spawn(move || background(worker));
            if spawned.is_err() {
                // Without a writer thread, save inline as before.
                let target = state.dirty;
                drop(state);
                let _ = commit(shared, target);
                return;
            }
            state.worker = true;
        }
        drop(state);
        shared.changed.notify_all();
    }

    /// Save now (together with concurrent callers); for steps that must not
    /// continue before the current mapping is on disk.
    pub(super) fn write_now(&self) -> io::Result<()> {
        if self.shared.path.is_none() {
            return Ok(());
        }
        let target = {
            let mut state = self.shared.lock();
            state.dirty += 1;
            state.dirty
        };
        commit(&self.shared, target)
    }

    /// Completed writes (successful or not).
    #[cfg(test)]
    pub(super) fn writes(&self) -> u64 {
        self.shared.lock().writes
    }
}

impl Drop for CacheStore {
    fn drop(&mut self) {
        let shared = &self.shared;
        if shared.path.is_none() {
            return;
        }
        let target = {
            let mut state = shared.lock();
            state.closing = true;
            state.dirty
        };
        shared.changed.notify_all();
        // The last clone of the backend is gone: pending changes go to disk.
        let _ = commit(shared, target);
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Counters only; a panic elsewhere leaves them consistent.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait_for<'a>(&self, state: MutexGuard<'a, State>, time: Duration) -> MutexGuard<'a, State> {
        self.changed
            .wait_timeout(state, time)
            .map(|(state, _)| state)
            .unwrap_or_else(|poisoned| poisoned.into_inner().0)
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        // Short snapshots under each lock; serialization and disk I/O run
        // without holding either map.
        let ids = snapshot(&self.ids)?;
        let mimes = snapshot(&self.mimes)?;
        super::cache::save_to_path(path, ids, mimes)
    }
}

fn snapshot(map: &Map) -> io::Result<HashMap<String, String>> {
    map.lock()
        .map(|map| map.clone())
        .map_err(|_| io::Error::other("Drive-Pfad-Cache vergiftet"))
}

/// Write every change up to `target`, or wait for a running write that
/// already contains it.
fn commit(shared: &Shared, target: u64) -> io::Result<()> {
    let mut state = shared.lock();
    loop {
        if state.written >= target {
            return Ok(());
        }
        if !state.writing {
            break;
        }
        state = shared.wait(state);
    }
    state.writing = true;
    let generation = state.dirty;
    drop(state);
    let started = Instant::now();
    let result = shared.save();
    let mut state = shared.lock();
    state.writing = false;
    state.writes += 1;
    state.last_cost = started.elapsed();
    if result.is_ok() {
        state.written = state.written.max(generation);
    }
    drop(state);
    shared.changed.notify_all();
    result
}

fn background(shared: Arc<Shared>) {
    loop {
        let mut state = shared.lock();
        while !state.closing && state.dirty <= state.written {
            state = shared.wait(state);
        }
        if state.closing {
            state.worker = false;
            return;
        }
        // Collect further changes for one interval (longer after costly
        // writes) before saving them all at once.
        let pause = shared
            .interval
            .max(state.last_cost.saturating_mul(COST_FACTOR));
        let due = Instant::now() + pause;
        loop {
            let now = Instant::now();
            if state.closing || now >= due {
                break;
            }
            state = shared.wait_for(state, due - now);
        }
        if state.closing {
            // `Drop` writes the pending changes itself.
            state.worker = false;
            return;
        }
        let target = state.dirty;
        drop(state);
        // A failed write stays dirty and is tried again after the interval.
        let _ = commit(&shared, target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps() -> (Map, Map) {
        (
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
        )
    }

    fn insert(map: &Map, from: usize, to: usize) {
        let mut map = map.lock().unwrap();
        for index in from..to {
            map.insert(format!("ordner/{index}"), format!("id-{index}"));
        }
    }

    #[test]
    fn transfer_engine_task_drive_path_cache_writes_are_bundled_and_flushed_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gdrive").join("path_cache.json");
        let (ids, mimes) = maps();
        let store = CacheStore::with_interval(
            Some(path.clone()),
            Arc::clone(&ids),
            Arc::clone(&mimes),
            Duration::from_secs(60),
        );
        for index in 0..500 {
            insert(&ids, index, index + 1);
            store.mark_dirty();
        }
        // 500 changes, no synchronous write: the writer waits for its interval.
        assert_eq!(store.writes(), 0);
        assert!(!path.exists());

        // A journal step that needs the mapping on disk writes all of it once.
        store.write_now().unwrap();
        assert_eq!(store.writes(), 1);
        assert_eq!(
            super::super::cache::load_from_path(&path)
                .unwrap()
                .ids
                .len(),
            500
        );
        store.write_now().unwrap();
        assert_eq!(store.writes(), 2);

        insert(&ids, 500, 800);
        store.mark_dirty();
        drop(store);
        let loaded = super::super::cache::load_from_path(&path).unwrap();
        assert_eq!(loaded.ids.len(), 800);
        assert_eq!(
            loaded.ids.get("ordner/799").map(String::as_str),
            Some("id-799")
        );
        // Compact JSON, no pretty-printing of large caches.
        assert!(!std::fs::read_to_string(&path).unwrap().contains('\n'));
    }

    #[test]
    fn transfer_engine_task_drive_path_cache_concurrent_journal_writes_share_one_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("path_cache.json");
        let (ids, mimes) = maps();
        let store = Arc::new(CacheStore::with_interval(
            Some(path.clone()),
            Arc::clone(&ids),
            mimes,
            Duration::from_secs(60),
        ));
        let workers: Vec<_> = (0..16)
            .map(|index| {
                let (store, ids) = (Arc::clone(&store), Arc::clone(&ids));
                std::thread::spawn(move || {
                    insert(&ids, index, index + 1);
                    store.write_now()
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        // Every caller returned only after a write that contained its entry.
        assert!(store.writes() <= 16);
        assert_eq!(
            super::super::cache::load_from_path(&path)
                .unwrap()
                .ids
                .len(),
            16
        );
    }
}
