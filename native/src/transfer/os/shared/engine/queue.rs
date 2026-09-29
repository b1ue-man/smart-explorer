//! Work found by discovery and not yet taken by a worker. The queue is
//! bounded by the memory its entries hold, not by a count: discovery runs far
//! ahead (the totals shown are known early) while a million-file tree never
//! sits in memory at once. It grows on demand; nothing is allocated upfront.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Queued entries may hold this much path text: about 100 000 files of a
/// typical path length, enough to know the totals of nearly every interactive
/// copy before the first files finish, while ten simultaneous million-file
/// jobs stay near 160 MiB.
const QUEUE_BYTES: usize = 16 * 1024 * 1024;
/// Bookkeeping per queued entry beyond its strings (struct, deque slot).
const ENTRY_OVERHEAD: usize = 96;
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// Packets look this far into the queue for more small files; a bounded scan
/// keeps taking a packet cheap even when the queue is long.
const BATCH_SCAN: usize = 4096;

/// One file to transfer, as discovery found it.
#[derive(Clone, Debug)]
pub(crate) struct FileWork {
    /// Path on the source side.
    pub source: String,
    /// Destination relative to the target folder (before root numbering).
    pub rel: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub id: Option<String>,
    pub md5: Option<String>,
    /// The one retry of a transient failure was used.
    pub retried: bool,
    /// Goes alone, never in a packet again (a packet ended before it).
    pub alone: bool,
    /// Since when the peer has refused this file as too busy (K13): it
    /// waits while the job still moves.
    pub overloaded_since: Option<Instant>,
}

impl FileWork {
    /// A file as discovery finds it.
    pub(crate) fn new(source: String, rel: String, size: u64, mtime_ms: i64) -> Self {
        Self {
            source,
            rel,
            size,
            mtime_ms,
            id: None,
            md5: None,
            retried: false,
            alone: false,
            overloaded_since: None,
        }
    }

    fn bytes(&self) -> usize {
        self.source.len()
            + self.rel.len()
            + self.id.as_ref().map_or(0, String::len)
            + self.md5.as_ref().map_or(0, String::len)
            + ENTRY_OVERHEAD
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Work {
    /// A folder of an unfiltered tree (kept even when empty) and its path
    /// on the source side (a move removes it once it is empty).
    Dir {
        rel: String,
        source: String,
    },
    File(FileWork),
}

impl Work {
    fn bytes(&self) -> usize {
        match self {
            Work::Dir { rel, source } => rel.len() + source.len() + ENTRY_OVERHEAD,
            Work::File(file) => file.bytes(),
        }
    }
}

#[derive(Default)]
struct State {
    items: VecDeque<Work>,
    bytes: usize,
    producing: bool,
    workers: usize,
    idle: usize,
}

/// Counts for the dispatcher's spawning decision.
#[derive(Clone, Copy, Debug)]
pub(crate) struct QueueView {
    pub queued: usize,
    pub workers: usize,
    pub idle: usize,
    pub producing: bool,
}

pub(crate) struct WorkQueue {
    state: Mutex<State>,
    changed: Condvar,
}

impl WorkQueue {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                producing: true,
                ..State::default()
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, State>, slice: Duration) -> MutexGuard<'a, State> {
        match self.changed.wait_timeout(guard, slice) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    /// Adds work, waiting while the queue is full; false once `stop` is set.
    pub(crate) fn push(&self, work: Work, stop: &AtomicBool) -> bool {
        let bytes = work.bytes();
        let mut state = self.lock();
        while state.bytes > 0 && state.bytes.saturating_add(bytes) > QUEUE_BYTES {
            if stop.load(Ordering::Acquire) {
                return false;
            }
            state = self.wait(state, WAIT_SLICE);
        }
        state.bytes = state.bytes.saturating_add(bytes);
        state.items.push_back(work);
        drop(state);
        self.changed.notify_all();
        true
    }

    /// Puts retried work back at the front, ignoring the bound (it was
    /// accounted before).
    pub(crate) fn push_front(&self, work: Work) {
        let mut state = self.lock();
        state.bytes = state.bytes.saturating_add(work.bytes());
        state.items.push_front(work);
        drop(state);
        self.changed.notify_all();
    }

    /// Discovery ended; workers drain what is left and stop.
    pub(crate) fn finish_producing(&self) {
        self.lock().producing = false;
        self.changed.notify_all();
    }

    /// Next work for a worker, waiting up to `linger` while the queue is
    /// empty but discovery still runs. `None` ends the worker.
    pub(crate) fn pop(&self, stop: &AtomicBool, linger: Duration) -> Option<Work> {
        let deadline = Instant::now() + linger;
        let mut state = self.lock();
        state.idle += 1;
        let next = loop {
            if stop.load(Ordering::Acquire) {
                break None;
            }
            if let Some(work) = state.items.pop_front() {
                state.bytes = state.bytes.saturating_sub(work.bytes());
                break Some(work);
            }
            let now = Instant::now();
            if !state.producing || now >= deadline {
                break None;
            }
            state = self.wait(state, (deadline - now).min(WAIT_SLICE));
        };
        state.idle -= 1;
        drop(state);
        self.changed.notify_all();
        next
    }

    /// Takes up to `max_files` more queued small files (each at most
    /// `small`) whose sizes fit into `max_bytes` together, for one packet.
    pub(crate) fn take_small(&self, max_files: usize, max_bytes: u64, small: u64) -> Vec<FileWork> {
        let mut taken = Vec::new();
        if max_files == 0 {
            return taken;
        }
        let mut budget = max_bytes;
        let mut state = self.lock();
        let mut index = 0;
        while index < state.items.len().min(BATCH_SCAN) && taken.len() < max_files {
            let fits = matches!(
                &state.items[index],
                Work::File(file)
                    if !file.retried && !file.alone && file.size <= small && file.size <= budget
            );
            if !fits {
                index += 1;
                continue;
            }
            if let Some(Work::File(file)) = state.items.remove(index) {
                state.bytes = state.bytes.saturating_sub(file.bytes());
                budget -= file.size;
                taken.push(file);
            }
        }
        drop(state);
        if !taken.is_empty() {
            self.changed.notify_all();
        }
        taken
    }

    /// Registers a worker the dispatcher is about to start; the returned
    /// slot counts it until dropped (also when the worker unwinds).
    pub(crate) fn add_worker(&self) -> WorkerSlot<'_> {
        self.lock().workers += 1;
        WorkerSlot { queue: self }
    }

    pub(crate) fn view(&self) -> QueueView {
        let state = self.lock();
        QueueView {
            queued: state.items.len(),
            workers: state.workers,
            idle: state.idle,
            producing: state.producing,
        }
    }

    /// Waits until something changed or `slice` passed.
    pub(crate) fn wait_changed(&self, slice: Duration) {
        let state = self.lock();
        drop(self.wait(state, slice));
    }

    pub(crate) fn notify(&self) {
        self.changed.notify_all();
    }

    /// Drops everything still queued (the job stops); returns how many files
    /// were never started.
    pub(crate) fn clear(&self) -> u64 {
        let mut state = self.lock();
        let files = state
            .items
            .iter()
            .filter(|work| matches!(work, Work::File(_)))
            .count() as u64;
        state.items.clear();
        state.bytes = 0;
        drop(state);
        self.changed.notify_all();
        files
    }
}

/// One running worker, counted while this lives.
pub(crate) struct WorkerSlot<'a> {
    queue: &'a WorkQueue,
}

impl Drop for WorkerSlot<'_> {
    fn drop(&mut self) {
        let mut state = self.queue.lock();
        state.workers = state.workers.saturating_sub(1);
        drop(state);
        self.queue.changed.notify_all();
    }
}
