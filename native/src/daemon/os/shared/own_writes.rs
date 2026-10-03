//! Written states, scoped to a job and side. Event consumers compare the
//! current state on their own thread before suppressing a write notification.
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use crate::bisync::{ApplySink, CompletedAction, CompletedKind, PairSide, Sig};
use crate::vfs::BackendHandle;

#[derive(Clone)]
enum Written {
    File(Sig),
    Directory,
    Missing,
}

#[derive(Clone)]
struct Entry {
    backend: BackendHandle,
    path: String,
    endpoint: String,
    generation: i64,
    succeeded: bool,
    state: Written,
}

type Key = (String, PairSide, String);
static WRITES: Mutex<BTreeMap<Key, Entry>> = Mutex::new(BTreeMap::new());
const MAX_WRITTEN_PATHS: usize = 32_768;

pub(super) struct Observer {
    pub(super) job_id: String,
    pub(super) a: BackendHandle,
    pub(super) b: BackendHandle,
    pub(super) root_a: String,
    pub(super) root_b: String,
    pub(super) source: String,
    pub(super) target: String,
    pub(super) generation: i64,
    pub(super) progress: std::sync::Arc<AtomicI64>,
}

impl Observer {
    fn record(&self, side: PairSide, rel: &str, state: Written) {
        let (backend, root, endpoint) = match side {
            PairSide::A => (&self.a, &self.root_a, &self.source),
            PairSide::B => (&self.b, &self.root_b, &self.target),
        };
        let path = if root.ends_with('/') { format!("{root}{rel}") }
            else { format!("{root}/{rel}") };
        let mut writes = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        writes.insert(
            (self.job_id.clone(), side, rel.to_string()),
            Entry { backend: backend.clone(), path, endpoint: endpoint.clone(), generation: self.generation, succeeded: false, state },
        );
        // Evicted entries simply retain their conservative follow-up run.
        while writes.len() > MAX_WRITTEN_PATHS { writes.pop_first(); }
    }
}

impl ApplySink for Observer {
    fn completed(&self, action: CompletedAction) {
        self.progress.store(super::state::now_secs(), Ordering::Release);
        match action.kind {
            CompletedKind::Copied { from } | CompletedKind::Moved { from } => {
                if let Some(sig) = action.dst_sig {
                    self.record(from.other(), &action.rel, Written::File(sig));
                }
                if matches!(action.kind, CompletedKind::Moved { .. }) {
                    self.record(from, &action.rel, Written::Missing);
                }
            }
            CompletedKind::Deleted { side } | CompletedKind::DirRemoved { side } => {
                self.record(side, &action.rel, Written::Missing);
            }
            CompletedKind::DirCreated { side } => {
                self.record(side, &action.rel, Written::Directory);
            }
        }
    }

    fn deferred(&self, _rel: &str, _reason: &str) {
        self.progress.store(super::state::now_secs(), Ordering::Release);
        super::job_triggers::persist(&self.job_id, crate::syncjobs::PendingKind::Verify, super::state::now_secs(), None);
    }
}

pub(super) fn forget(id: &str) {
    WRITES.lock().unwrap_or_else(PoisonError::into_inner)
        .retain(|(job, _, _), _| job != id);
}

pub(super) fn succeeded(id: &str, generation: i64) {
    for ((job, _, _), entry) in WRITES.lock().unwrap_or_else(PoisonError::into_inner).iter_mut() {
        if job == id && entry.generation == generation { entry.succeeded = true; }
    }
}

pub(super) fn candidate(id: &str, side: PairSide, rel: &str, endpoint: &str, generation: Option<i64>) -> bool {
    WRITES.lock().unwrap_or_else(PoisonError::into_inner).get(&(id.to_string(), side, rel.to_string()))
        .is_some_and(|entry| entry.endpoint == endpoint && generation == Some(entry.generation)
            && match &entry.state { Written::Missing => true, Written::File(sig) => sig.hash != 0, Written::Directory => false })
}

/// A failed stat is never interpreted as missing (or as an own write).
pub(super) fn matches(id: &str, side: PairSide, rel: &str, endpoint: &str,
    generation: Option<i64>, cancel: &AtomicBool) -> bool {
    let key = (id.to_string(), side, rel.to_string());
    let entry = WRITES.lock().unwrap_or_else(PoisonError::into_inner).get(&key).cloned();
    let Some(entry) = entry else { return false; };
    if !entry.succeeded || entry.endpoint != endpoint || generation != Some(entry.generation) { return false; }
    let equal = match (entry.state, entry.backend.stat(&entry.path)) {
        (Written::Missing, Err(error)) => error.kind() == std::io::ErrorKind::NotFound,
        (Written::Directory, _) => false,
        (Written::File(sig), Ok(meta)) => !meta.is_dir && !meta.is_symlink && !meta.special
            && sig.hash != 0 && meta.size == sig.size && meta.mtime_ms == sig.mtime_ms
            && crate::bisync::current_content_signature(&*entry.backend, &entry.path, cancel).ok() == Some(sig.hash)
            && entry.backend.stat(&entry.path).is_ok_and(|after| after.size == sig.size && after.mtime_ms == sig.mtime_ms
                && !after.is_dir && !after.is_symlink && !after.special),
        _ => false,
    };
    if !equal {
        WRITES.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
    }
    equal
}

#[cfg(test)]
#[path = "own_writes_tests.rs"]
mod tests;
