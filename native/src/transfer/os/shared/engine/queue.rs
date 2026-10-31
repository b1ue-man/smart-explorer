//! Work found by discovery and not yet taken by a worker. The queue is
//! bounded by the memory its entries hold, not by a count: discovery runs far
//! ahead (the totals shown are known early) while a million-file tree never
//! sits in memory at once. It grows on demand; nothing is allocated upfront.
//!
//! Files go before folders: a folder with files is created by its first file
//! (the folder register), so a listing's folders never hold back the first
//! files (twenty folders at 50 ms cost a second before any file otherwise),
//! and folder work only matters for empty folders and a move's cleanup.
//! While discovery runs, a worker takes a folder only after it lingered
//! without a file (or when the queue is half full), so the files found
//! meanwhile create nested folders in one bulk request. Where the target
//! creates folders in bulk, a worker that comes for folder work takes all
//! queued folders (up to one packet's worth) in one go, so they are created
//! together with whatever the files ask for at the same time.
//!
//! For a target that takes packets, the first files wait for more company
//! while discovery runs (until a packet's worth is queued, discovery ends or
//! `GATHER` passed since the first file): a worker that is idle takes each
//! file the instant it is found, which is faster than discovery on some
//! systems, so without the wait a small tree would travel as single files
//! on them and as one packet on the others.
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
/// How long the first files of a job for a packet target wait for more
/// company (one worker linger: longer than a listing round trip on common
/// links, short against anything a user sees).
const GATHER: Duration = Duration::from_millis(250);
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
    /// Stable explicit-pair identity, independent of top-level numbering.
    pub receipt_key: Option<String>,
    /// The one retry of a transient failure was used.
    pub retried: bool,
    /// Goes alone, never in a packet again (a packet ended before it).
    pub alone: bool,
    /// Since when the peer has refused this file as too busy (K13): it
    /// waits while the job still moves.
    pub overloaded_since: Option<Instant>,
    /// A one-request publication under the planned name found it taken
    /// (G3-08b): this file takes the staged path, which numbers the name.
    pub absent_refused: bool,
    /// The conflict policy the file rule "neuere ersetzen ältere" decided for
    /// this file (`update.rs`): `Rename` where the target is missing or
    /// the files conflict, `Overwrite` where a newer source replaces. `None`:
    /// the job's own policy applies.
    pub conflict: Option<crate::types::Conflict>,
    /// The file replaces an existing target: what the rule saw of it (size,
    /// time, and the provider id when the listing gave one; duplicate-capable
    /// providers bind by it). The target is compared with it once more right
    /// before it is replaced.
    pub replacing: Option<crate::transfer::update_rule::Replacing>,
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
            receipt_key: None,
            retried: false,
            alone: false,
            overloaded_since: None,
            absent_refused: false,
            conflict: None,
            replacing: None,
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
    /// Folders `(rel, source)` taken together by one worker (see `pop`).
    Dirs(Vec<(String, String)>),
    /// A ticket for one more helper of a split large file (`segments`): it
    /// asks a worker to take parts of the file `job` is transferring. Lives
    /// in the files lane; packets never take it.
    Segment {
        job: u64,
    },
}

impl Work {
    fn bytes(&self) -> usize {
        match self {
            Work::Dir { rel, source } => rel.len() + source.len() + ENTRY_OVERHEAD,
            Work::Dirs(dirs) => dirs
                .iter()
                .map(|(rel, source)| rel.len() + source.len() + ENTRY_OVERHEAD)
                .sum(),
            Work::File(file) => file.bytes(),
            Work::Segment { .. } => ENTRY_OVERHEAD,
        }
    }
}

#[derive(Default)]
struct State {
    /// Files, in discovery order.
    items: VecDeque<Work>,
    /// Folders, taken when no file is queued.
    dirs: VecDeque<Work>,
    bytes: usize,
    producing: bool,
    workers: usize,
    idle: usize,
    /// Files per packet the job's target takes; 0: no packets, nothing waits.
    gather_files: usize,
    /// Until when the first files wait for more company; set by the first
    /// file found.
    gather_until: Option<Instant>,
}

impl State {
    fn lane(&mut self, work: &Work) -> &mut VecDeque<Work> {
        match work {
            Work::Dir { .. } | Work::Dirs(_) => &mut self.dirs,
            Work::File(_) | Work::Segment { .. } => &mut self.items,
        }
    }

    /// Whether the files wait now for more company: the target takes
    /// packets, discovery still runs, less than a packet's worth is queued
    /// and the first file was found less than `GATHER` ago.
    fn gathering(&self, now: Instant) -> bool {
        self.producing
            && self.items.len() < self.gather_files
            && self.gather_until.is_some_and(|until| now < until)
    }

    /// The next folder work: one folder, or, where `group` is set (the
    /// target creates folders in bulk), all queued folders up to a packet's
    /// worth at once, so no other worker takes a part of them and creates it
    /// with a request of its own.
    fn next_folders(&mut self, group: bool) -> Option<Work> {
        if !group || self.gather_files < 2 || self.dirs.len() < 2 {
            return self.dirs.pop_front();
        }
        let count = self.dirs.len().min(self.gather_files);
        let dirs = self
            .dirs
            .drain(..count)
            .filter_map(|work| match work {
                Work::Dir { rel, source } => Some((rel, source)),
                _ => None,
            })
            .collect();
        Some(Work::Dirs(dirs))
    }
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
    /// A queue for a job whose target takes packets of `gather_files` files
    /// (0: no packets).
    pub(crate) fn new(gather_files: usize) -> Self {
        Self {
            state: Mutex::new(State {
                producing: true,
                gather_files,
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
        if state.gather_files > 0 && state.gather_until.is_none() && matches!(work, Work::File(_)) {
            state.gather_until = Some(Instant::now() + GATHER);
        }
        state.lane(&work).push_back(work);
        drop(state);
        self.changed.notify_all();
        true
    }

    /// Puts retried work back at the front, ignoring the bound (it was
    /// accounted before).
    pub(crate) fn push_front(&self, work: Work) {
        let mut state = self.lock();
        state.bytes = state.bytes.saturating_add(work.bytes());
        state.lane(&work).push_front(work);
        drop(state);
        self.changed.notify_all();
    }

    /// Discovery ended; workers drain what is left and stop.
    pub(crate) fn finish_producing(&self) {
        self.lock().producing = false;
        self.changed.notify_all();
    }

    /// Next work for a worker, waiting up to `linger` while the queue is
    /// empty but discovery still runs. `None` ends the worker. Where
    /// `group_dirs` is set, folder work comes as all queued folders at once
    /// (`Work::Dirs`).
    pub(crate) fn pop(
        &self,
        stop: &AtomicBool,
        linger: Duration,
        group_dirs: bool,
    ) -> Option<Work> {
        let deadline = Instant::now() + linger;
        let mut state = self.lock();
        state.idle += 1;
        let next = loop {
            if stop.load(Ordering::Acquire) {
                break None;
            }
            let now = Instant::now();
            // The first files of a packet job wait for more company, so a
            // packet forms however fast discovery is compared to a worker.
            let gathering = state.gathering(now);
            // A folder waits while discovery may still bring its files (at
            // most one linger, or until the queue is half full): its first
            // file then creates it with all its missing parents in one
            // request where the target creates folders in bulk.
            let folders_due = !state.producing || now >= deadline || state.bytes >= QUEUE_BYTES / 2;
            let file = if gathering {
                None
            } else {
                state.items.pop_front()
            };
            let next = match file {
                Some(work) => Some(work),
                None if folders_due => state.next_folders(group_dirs),
                None => None,
            };
            if let Some(work) = next {
                state.bytes = state.bytes.saturating_sub(work.bytes());
                break Some(work);
            }
            if !state.producing || (now >= deadline && !gathering) {
                break None;
            }
            let until = match state.gather_until {
                Some(until) if gathering => until,
                _ => deadline,
            };
            state = self.wait(state, until.saturating_duration_since(now).min(WAIT_SLICE));
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
            queued: state.items.len() + state.dirs.len(),
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
        state.dirs.clear();
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
