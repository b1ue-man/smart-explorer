//! Background uploads of closed dirty files (G4 ML1).
//!
//! `close` (FUSE `flush`, Dokany `Cleanup`) returns once the spool content is
//! durable and its `Dirty` journal record exists; the entry then waits here.
//! Workers (`mount-upload`, at most the backend's request capacity, started
//! on demand) publish entries through the unchanged save path. Until a save
//! is confirmed, the journal entry stays the source of truth: a crash or an
//! unmount with a failed upload leaves it `Dirty`, and the next mount or
//! Retry uploads it again. A failed upload is tried again after 1 s, 2 s,
//! 4 s … at most 60 s, or at once when the file is closed again; this is a
//! retry pause, not a deadline that fails anything.
use super::engine::{lock, Entry, MountEngine};
use super::types::EntryCondition;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(60);

/// The status of a drive's background uploads, as the host reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MountUploadStatus {
    /// Closed files waiting for their upload, including those that wait for
    /// a retry.
    pub pending_files: u64,
    /// Their content in bytes, as it was when the file was closed.
    pub pending_bytes: u64,
    /// Uploads running now.
    pub uploading: u64,
    /// Files whose last upload failed and that wait for their retry.
    pub retrying: u64,
    /// The newest upload failure; cleared by the next successful upload.
    pub last_error: Option<String>,
}

impl MountUploadStatus {
    /// The lines the CLI and the desktop show for the uploads; each appears
    /// only while files are pending, uploading, retrying or after a failure.
    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.pending_files > 0 {
            let files = if self.pending_files == 1 {
                "Datei"
            } else {
                "Dateien"
            };
            lines.push(format!(
                "Uploads ausstehend: {} {files} ({})",
                self.pending_files,
                crate::format::format_bytes(self.pending_bytes)
            ));
        }
        if self.uploading > 0 {
            lines.push(format!("lädt hoch: {}", self.uploading));
        }
        if self.retrying > 0 {
            lines.push(format!("Wiederholung: {}", self.retrying));
        }
        if let Some(error) = &self.last_error {
            lines.push(format!("Letzter Fehler: {error}"));
        }
        lines
    }
}

/// What one upload attempt ended with.
pub(super) enum Uploaded {
    Done,
    Conflict(String, EntryCondition),
    /// The file changed while it was uploading; the new content goes next.
    Changed,
    Failed(String),
}

/// The engine's handle on its queue; dropping the engine ends the workers.
#[derive(Default)]
pub(super) struct WriteBack {
    queue: Arc<UploadQueue>,
}

impl WriteBack {
    pub(super) fn queue(&self) -> &Arc<UploadQueue> {
        &self.queue
    }
}

impl Drop for WriteBack {
    fn drop(&mut self) {
        self.queue.close();
    }
}

#[derive(Default)]
pub(super) struct UploadQueue {
    state: Mutex<QueueState>,
    work: Condvar,
    idle: Condvar,
}

#[derive(Default)]
struct QueueState {
    engine: Option<Weak<MountEngine>>,
    /// Workers allowed at once; 0 = no write-back (saves stay synchronous).
    limit: usize,
    workers: usize,
    idle_workers: usize,
    closed: bool,
    pending: VecDeque<Arc<Entry>>,
    queued: HashSet<usize>,
    active: HashSet<usize>,
    /// Closed again while uploading: upload once more afterwards.
    again: HashSet<usize>,
    delayed: Vec<(Instant, Arc<Entry>)>,
    attempts: HashMap<usize, u32>,
    bytes: HashMap<usize, u64>,
    last_error: Option<String>,
    conflicts: Vec<(String, EntryCondition)>,
}

fn entry_key(entry: &Arc<Entry>) -> usize {
    Arc::as_ptr(entry) as usize
}

impl QueueState {
    fn push(&mut self, entry: Arc<Entry>) {
        let id = entry_key(&entry);
        if self.active.contains(&id) {
            self.again.insert(id);
        } else if self.queued.insert(id) {
            self.delayed.retain(|(_, delayed)| entry_key(delayed) != id);
            self.pending.push_back(entry);
        }
    }

    fn promote_due(&mut self, now: Instant) {
        let mut index = 0;
        while index < self.delayed.len() {
            if self.delayed[index].0 <= now {
                let (_, entry) = self.delayed.swap_remove(index);
                self.pending.push_back(entry);
            } else {
                index += 1;
            }
        }
    }

    fn take(&mut self, entry: Arc<Entry>) -> Arc<Entry> {
        let id = entry_key(&entry);
        self.queued.remove(&id);
        self.active.insert(id);
        entry
    }
}

impl UploadQueue {
    /// Lets up to `limit` workers upload for `engine` from now on.
    pub(super) fn attach(&self, engine: Weak<MountEngine>, limit: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.engine = Some(engine);
            state.limit = limit.max(1);
        }
    }

    /// Whether `close` may return before the upload (workers attached).
    pub(super) fn accepts(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.limit > 0 && !state.closed)
    }

    /// Queues `entry`; `false` when write-back is off and the caller must
    /// save synchronously.
    pub(super) fn enqueue(self: &Arc<Self>, entry: Arc<Entry>, bytes: u64) -> bool {
        let spawn = {
            let Ok(mut state) = self.state.lock() else {
                return false;
            };
            if state.limit == 0 || state.closed {
                return false;
            }
            let id = entry_key(&entry);
            state.bytes.insert(id, bytes);
            state.push(entry);
            let spawn = state.idle_workers == 0 && state.workers < state.limit;
            if spawn {
                state.workers += 1;
            }
            spawn.then(|| state.engine.clone()).flatten()
        };
        if let Some(engine) = spawn {
            let queue = Arc::clone(self);
            let started = std::thread::Builder::new()
                .name("mount-upload".into())
                .spawn(move || worker(queue, engine));
            if started.is_err() {
                if let Ok(mut state) = self.state.lock() {
                    state.workers = state.workers.saturating_sub(1);
                }
            }
        }
        self.work.notify_one();
        true
    }

    /// The next entry to upload; `None` once the queue is closed.
    fn next(&self) -> Option<Arc<Entry>> {
        let mut state = self.state.lock().ok()?;
        loop {
            if state.closed {
                return None;
            }
            let now = Instant::now();
            state.promote_due(now);
            if let Some(entry) = state.pending.pop_front() {
                return Some(state.take(entry));
            }
            let due = state.delayed.iter().map(|(due, _)| *due).min();
            state.idle_workers += 1;
            state = match due {
                Some(due) => {
                    let pause = due.saturating_duration_since(now);
                    self.work.wait_timeout(state, pause).ok()?.0
                }
                None => self.work.wait(state).ok()?,
            };
            state.idle_workers -= 1;
        }
    }

    /// Records the end of one upload of `entry`.
    pub(super) fn done(&self, entry: &Arc<Entry>, outcome: Uploaded) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let id = entry_key(entry);
        state.active.remove(&id);
        let mut again = state.again.remove(&id);
        match outcome {
            Uploaded::Done => {
                state.attempts.remove(&id);
                state.last_error = None;
            }
            Uploaded::Conflict(path, condition) => {
                state.attempts.remove(&id);
                state.conflicts.push((path, condition));
            }
            Uploaded::Changed => again = true,
            Uploaded::Failed(error) => {
                state.last_error = Some(error);
                let count = {
                    let attempts = state.attempts.entry(id).or_insert(0);
                    *attempts = attempts.saturating_add(1);
                    *attempts
                };
                let pause = FIRST_RETRY
                    .saturating_mul(1u32 << (count - 1).min(6))
                    .min(LAST_RETRY);
                if !again && !state.closed {
                    state
                        .delayed
                        .push((Instant::now() + pause, Arc::clone(entry)));
                }
            }
        }
        if again {
            state.push(Arc::clone(entry));
        }
        if !state.queued.contains(&id)
            && !state.active.contains(&id)
            && !state
                .delayed
                .iter()
                .any(|(_, delayed)| entry_key(delayed) == id)
        {
            state.bytes.remove(&id);
        }
        drop(state);
        self.work.notify_one();
        self.idle.notify_all();
    }

    /// Ends background work: workers stop after their current upload.
    pub(super) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.work.notify_all();
    }

    /// After `close`: every entry still waiting, retries included, at once.
    pub(super) fn take_for_drain(&self) -> Option<Arc<Entry>> {
        let mut state = self.state.lock().ok()?;
        if let Some(entry) = state.pending.pop_front() {
            return Some(state.take(entry));
        }
        let (_, entry) = state.delayed.pop()?;
        Some(state.take(entry))
    }

    /// Waits until no upload runs any more.
    pub(super) fn wait_idle(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        while !state.active.is_empty() {
            state = match self.idle.wait(state) {
                Ok(state) => state,
                Err(_) => return,
            };
        }
    }

    /// `None` without write-back.
    pub(super) fn status(&self) -> Option<MountUploadStatus> {
        let state = self.state.lock().ok()?;
        if state.limit == 0 {
            return None;
        }
        Some(MountUploadStatus {
            pending_files: (state.pending.len() + state.delayed.len() + state.active.len()) as u64,
            pending_bytes: state.bytes.values().copied().sum(),
            uploading: state.active.len() as u64,
            retrying: state.delayed.len() as u64,
            last_error: state.last_error.clone(),
        })
    }

    pub(super) fn take_conflicts(&self) -> Vec<(String, EntryCondition)> {
        self.state
            .lock()
            .map(|mut state| std::mem::take(&mut state.conflicts))
            .unwrap_or_default()
    }
}

/// One worker: uploads until the queue closes or the engine is gone.
fn worker(queue: Arc<UploadQueue>, engine: Weak<MountEngine>) {
    while let Some(entry) = queue.next() {
        let outcome = match engine.upgrade() {
            Some(engine) => engine.upload_once(&entry),
            None => Uploaded::Failed("the drive was closed".into()),
        };
        queue.done(&entry, outcome);
    }
    if let Ok(mut state) = queue.state.lock() {
        state.workers = state.workers.saturating_sub(1);
    }
}

impl MountEngine {
    /// Whether the entry still holds an unsaved change.
    pub(super) fn still_dirty(entry: &Arc<Entry>) -> io::Result<bool> {
        Ok(lock(&entry.state)?.condition == EntryCondition::Dirty)
    }
}

#[cfg(test)]
#[path = "upload_queue_task_tests.rs"]
mod task_tests;
