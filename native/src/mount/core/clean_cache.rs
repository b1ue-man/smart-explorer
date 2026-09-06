//! Disposable in-session content, never live Entry values or journal state.
use super::engine::lock;
use super::spool::WholeFileSpool;
use super::types::Baseline;
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) const MAX_IDLE_RECORDS: usize = 10_000;
pub(super) const MAX_CONTENT_AGE: Duration = Duration::from_secs(300);

pub(super) struct IdleClean {
    pub path: String,
    pub key: String,
    pub spool_name: String,
    pub baseline: Baseline,
    pub bytes: u64,
    pub created: Instant,
    pub reusable: bool,
    touched: Instant,
}

impl IdleClean {
    pub fn new(
        path: String,
        key: String,
        spool_name: String,
        baseline: Baseline,
        bytes: u64,
        created: Instant,
    ) -> Self {
        Self {
            path,
            key,
            spool_name,
            baseline,
            bytes,
            created,
            reusable: true,
            touched: Instant::now(),
        }
    }
}

#[derive(Default)]
struct State {
    records: HashMap<String, IdleClean>,
    paths: HashMap<String, String>,
    // Invalid records are reclaimed first, then least-recently retired content.
    lru: BTreeSet<(bool, Instant, String)>,
    expiry: BTreeSet<(Instant, String)>,
    bytes: u128,
}

impl State {
    fn remove(&mut self, name: &str) -> Option<IdleClean> {
        let record = self.records.remove(name)?;
        if self.paths.get(&record.key).is_some_and(|current| current == name) {
            self.paths.remove(&record.key);
        }
        self.lru.remove(&(record.reusable, record.touched, record.spool_name.clone()));
        self.expiry.remove(&(record.created, record.spool_name.clone()));
        self.bytes -= u128::from(record.bytes);
        Some(record)
    }

    fn invalidate_name(&mut self, name: &str) {
        let Some(record) = self.records.get_mut(name) else { return; };
        self.lru.remove(&(record.reusable, record.touched, record.spool_name.clone()));
        record.reusable = false;
        self.lru.insert((false, record.touched, record.spool_name.clone()));
        if self.paths.get(&record.key).is_some_and(|current| current == name) {
            self.paths.remove(&record.key);
        }
    }

    fn expire(&mut self) {
        while let Some((created, name)) = self.expiry.first().cloned() {
            if created.elapsed() < MAX_CONTENT_AGE {
                break;
            }
            self.expiry.remove(&(created, name.clone()));
            self.invalidate_name(&name);
        }
    }

    fn evict_first(&mut self, spool: &WholeFileSpool) -> io::Result<bool> {
        let Some((_, _, name)) = self.lru.first().cloned() else { return Ok(false); };
        // No bookkeeping changes until physical disposal succeeds.
        spool.remove_file(&name)?;
        self.remove(&name);
        Ok(true)
    }
}

#[derive(Default)]
pub(super) struct CleanCache {
    state: Mutex<State>,
}

impl CleanCache {
    pub fn claim(&self, key: &str) -> io::Result<Option<IdleClean>> {
        let mut state = lock(&self.state)?;
        let name = state.paths.get(key).cloned();
        Ok(name.and_then(|name| state.remove(&name)))
    }

    pub fn retain(&self, record: IdleClean) -> io::Result<()> {
        let mut state = lock(&self.state)?;
        if record.reusable {
            if let Some(previous) = state.paths.get(&record.key).cloned() {
                state.invalidate_name(&previous);
            }
        }
        state.remove(&record.spool_name);
        if record.reusable {
            state.paths.insert(record.key.clone(), record.spool_name.clone());
        }
        state.bytes += u128::from(record.bytes);
        state.lru.insert((record.reusable, record.touched, record.spool_name.clone()));
        state.expiry.insert((record.created, record.spool_name.clone()));
        state.records.insert(record.spool_name.clone(), record);
        Ok(())
    }

    pub fn invalidate(&self, key: &str, recursive: bool) -> io::Result<()> {
        let mut state = lock(&self.state)?;
        let names = if recursive {
            state.paths.iter().filter(|(path, _)| affected(path, key, true))
                .map(|(_, name)| name.clone()).collect::<Vec<_>>()
        } else {
            state.paths.get(key).cloned().into_iter().collect()
        };
        for name in names {
            state.invalidate_name(&name);
        }
        Ok(())
    }

    pub fn evict_path(&self, spool: &WholeFileSpool, key: &str) -> io::Result<bool> {
        let mut state = lock(&self.state)?;
        let names = state.records.values().filter(|record| record.key == key)
            .map(|record| record.spool_name.clone()).collect::<Vec<_>>();
        for name in &names {
            spool.remove_file(name)?;
            state.remove(name);
        }
        Ok(!names.is_empty())
    }

    pub fn evict_oldest(&self, spool: &WholeFileSpool) -> io::Result<bool> {
        lock(&self.state)?.evict_first(spool)
    }

    pub fn trim(&self, spool: &WholeFileSpool, limit: u64) -> io::Result<()> {
        let mut state = lock(&self.state)?;
        state.expire();
        while (limit == 0 && !state.records.is_empty())
            || state.records.len() > MAX_IDLE_RECORDS
            || state.bytes > u128::from(limit)
            || state.lru.first().is_some_and(|(valid, _, _)| !valid)
        {
            if !state.evict_first(spool)? {
                break;
            }
        }
        Ok(())
    }

    pub fn usage(&self) -> io::Result<(usize, u64)> {
        let state = lock(&self.state)?;
        Ok((state.records.len(), u64::try_from(state.bytes).unwrap_or(u64::MAX)))
    }
}

pub(super) fn affected(path: &str, key: &str, recursive: bool) -> bool {
    path == key || (recursive && path.strip_prefix(key.trim_end_matches('/'))
        .is_some_and(|suffix| suffix.starts_with('/')))
}
