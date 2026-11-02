//! Read-ahead listings of a copy expansion (`collect.rs`). Listing threads read
//! the folders the walk reaches soon. The walk takes each result at its own
//! turn and in its own order, so entries, messages, budgets and access requests
//! are those of a walk that reads every folder itself. A listing that failed, a
//! folder above `MAX_AHEAD_ENTRIES`, and a folder no thread has started are read
//! by the walk at its turn.
//!
//! Race semantics: a listing is a snapshot of its folder at the moment it is
//! read. A folder that changes between its read-ahead listing and its turn in
//! the walk shows the state of the read-ahead. The serial walk has the same
//! window between a folder's listing and its turn, only at a different point in
//! time. Every listing is a snapshot on both paths, and no guarantee of the copy
//! expansion changes with the read-ahead.
//!
//! Early end: when the walk ends on any path, `Close` sets `stop`. A listing that
//! has not started is skipped. A listing that waits for a device lock sees `stop`
//! through the cancellation scope of its thread (`serve`, `with_cancellation`),
//! and ends too. The caller's cancel flag is not touched.
use crate::local_access::LocalEntry;
use crate::source_media::{with_cancellation, GuardedIterator, Priority, WalkDevice};
use std::cell::Cell;
use std::collections::VecDeque;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// Entries one read-ahead listing may hold. A larger folder is read by the walk.
pub(super) const MAX_AHEAD_ENTRIES: usize = 4096;
/// Read-ahead listings per listing thread that the walk may hold: queued,
/// running, or done and not yet taken.
const AHEAD_PER_THREAD: usize = 2;
/// The most listing threads. The remote folder walk (`rscan`) caps its listers
/// at the same count.
const MAX_LISTING_THREADS: usize = 16;

/// The entries of one folder's listing, in listing order.
pub(super) type Items<'a> = Box<dyn Iterator<Item = io::Result<LocalEntry>> + 'a>;

/// A folder's listing as the walk takes it at its turn.
pub(super) enum Read {
    /// The folder's device and all of its entries, read ahead.
    Listed(WalkDevice, Vec<io::Result<LocalEntry>>),
    /// The walk reads the folder itself: the read-ahead failed, the folder is
    /// above `MAX_AHEAD_ENTRIES`, or no thread started it.
    Reread,
}

/// Opens a folder the way the walk does: its device first, then its entries
/// under the device's guard.
pub(super) fn open<'a>(
    directory: &Path,
    parent: Option<&WalkDevice>,
    follow_symlinks: bool,
    cancel: &'a AtomicBool,
) -> io::Result<(WalkDevice, Items<'a>)> {
    #[cfg(test)]
    if let Some(error) = injected(directory) {
        return Err(error);
    }
    let device = match parent {
        Some(parent) => parent.child(directory, follow_symlinks),
        None => WalkDevice::root(directory),
    }?;
    let read = GuardedIterator::with_device(
        directory,
        device.protected_source(),
        Priority::Foreground,
        cancel,
        || crate::local_access::read_directory(directory),
    )?;
    let items: Items<'a> = Box::new(read);
    Ok((device, items))
}

/// Folders whose opening fails with the given kind (tests only). The walk meets
/// such a fault at its turn, in the serial and the read-ahead paths alike.
#[cfg(test)]
pub(super) static INJECTED: Mutex<Vec<(PathBuf, io::ErrorKind)>> = Mutex::new(Vec::new());

/// Compared in the form the walk opens folders in (`normalize_scan_root`:
/// `\\?\C:\...` on Windows, unchanged elsewhere), whatever form the test
/// named the folder in.
#[cfg(test)]
fn injected(directory: &Path) -> Option<io::Error> {
    let opened = crate::local_access::normalize_scan_root(directory);
    lock(&INJECTED)
        .iter()
        .find(|(path, _)| crate::local_access::normalize_scan_root(path) == opened)
        .map(|(_, kind)| io::Error::from(*kind))
}

/// The listing threads of a copy expansion: the cores available at run time, as
/// the local recursive view uses them, at most `MAX_LISTING_THREADS`.
pub(super) fn threads() -> usize {
    super::super::walk::scan_threads().min(MAX_LISTING_THREADS)
}

/// What the walk asks of the listing threads.
pub(super) struct Listers<'a> {
    queue: &'a Queue,
    cancel: &'a AtomicBool,
    follow_symlinks: bool,
    /// Listing threads that started; 0 means the walk reads every folder itself.
    threads: usize,
    /// Read-ahead listings asked for and not yet taken by the walk.
    outstanding: Cell<usize>,
}

/// One folder's read-ahead listing: queued, running, or done.
pub(super) struct Ahead(Arc<Slot>);

struct Slot {
    stage: Mutex<Stage>,
    done: Condvar,
}

enum Stage {
    Queued(Job),
    Running,
    Done(Read),
    /// The walk took the folder itself, or the result was taken.
    Taken,
}

struct Job {
    directory: PathBuf,
    parent: Option<WalkDevice>,
    follow_symlinks: bool,
}

struct Queue {
    state: Mutex<Waiting>,
    ready: Condvar,
}

struct Waiting {
    jobs: VecDeque<Arc<Slot>>,
    closed: bool,
}

/// Runs `body` with up to `threads` listing threads (at most
/// `MAX_LISTING_THREADS`). The threads have ended when this returns, also when
/// `body` unwinds.
pub(super) fn run<R>(
    threads: usize,
    follow_symlinks: bool,
    cancel: &AtomicBool,
    body: impl FnOnce(&Listers<'_>) -> R,
) -> R {
    let queue = Queue {
        state: Mutex::new(Waiting {
            jobs: VecDeque::new(),
            closed: false,
        }),
        ready: Condvar::new(),
    };
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let mut started = 0;
        for index in 0..threads.min(MAX_LISTING_THREADS) {
            let queue = &queue;
            let stop = &stop;
            let spawned = std::thread::Builder::new()
                .name(format!("collect-listing-{index}"))
                .spawn_scoped(scope, move || serve(queue, cancel, stop));
            if spawned.is_ok() {
                started += 1;
            }
        }
        let _close = Close {
            queue: &queue,
            stop: &stop,
        };
        let listers = Listers {
            queue: &queue,
            cancel,
            follow_symlinks,
            threads: started,
            outstanding: Cell::new(0),
        };
        body(&listers)
    })
}

impl Listers<'_> {
    /// How many folders from the top of the walk's stack ask for a read-ahead.
    pub(super) fn window(&self) -> usize {
        self.threads
    }

    /// Queues the listing of `directory`. `None` when no listing thread started
    /// or the read-ahead budget is spent.
    pub(super) fn read_ahead(
        &self,
        directory: &Path,
        parent: Option<&WalkDevice>,
    ) -> Option<Ahead> {
        if self.outstanding.get() >= self.threads * AHEAD_PER_THREAD {
            return None;
        }
        let job = Job {
            directory: directory.to_path_buf(),
            parent: parent.cloned(),
            follow_symlinks: self.follow_symlinks,
        };
        let slot = Arc::new(Slot {
            stage: Mutex::new(Stage::Queued(job)),
            done: Condvar::new(),
        });
        lock(&self.queue.state).jobs.push_back(Arc::clone(&slot));
        self.queue.ready.notify_one();
        self.outstanding.set(self.outstanding.get() + 1);
        Some(Ahead(slot))
    }

    /// The walk's turn for `ahead`: the listing a thread read, or the walk's own
    /// read when no thread has started the folder. A running listing is awaited.
    pub(super) fn claim(&self, ahead: Ahead) -> Read {
        self.outstanding
            .set(self.outstanding.get().saturating_sub(1));
        let slot = ahead.0;
        let mut stage = lock(&slot.stage);
        loop {
            match std::mem::replace(&mut *stage, Stage::Taken) {
                Stage::Done(read) => return read,
                Stage::Queued(job) => {
                    // No thread has started it, so the walk reads it now.
                    drop(stage);
                    return list(&job, self.cancel);
                }
                Stage::Running => {
                    *stage = Stage::Running;
                    stage = slot
                        .done
                        .wait(stage)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Stage::Taken => return Read::Reread,
            }
        }
    }
}

/// Ends the listing threads on every exit of the walk: `stop` reaches the
/// listings in progress, and the queue closes so that the threads return.
struct Close<'a> {
    queue: &'a Queue,
    stop: &'a AtomicBool,
}

impl Drop for Close<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        lock(&self.queue.state).closed = true;
        self.queue.ready.notify_all();
    }
}

impl Queue {
    /// The next queued folder, or `None` once the queue has closed.
    fn pop(&self) -> Option<Arc<Slot>> {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return None;
            }
            if let Some(slot) = state.jobs.pop_front() {
                return Some(slot);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Slot {
    fn finish(&self, read: Read) {
        *lock(&self.stage) = Stage::Done(read);
        self.done.notify_all();
    }
}

/// A listing thread: takes queued folders until the queue closes. A listing runs
/// in a cancellation scope on `stop`, so the end of the walk reaches a device
/// lock wait inside it. The caller's cancel flag is passed on unchanged.
fn serve(queue: &Queue, cancel: &AtomicBool, stop: &AtomicBool) {
    while let Some(slot) = queue.pop() {
        let job = {
            let mut stage = lock(&slot.stage);
            match std::mem::replace(&mut *stage, Stage::Running) {
                Stage::Queued(job) => job,
                // The walk took this folder itself.
                taken => {
                    *stage = taken;
                    continue;
                }
            }
        };
        let read = catch_unwind(AssertUnwindSafe(|| {
            if stop.load(Ordering::Acquire) {
                // The walk has ended: nobody takes this listing any more.
                Read::Reread
            } else {
                with_cancellation(stop, || list(&job, cancel))
            }
        }))
        .unwrap_or(Read::Reread);
        slot.finish(read);
    }
}

/// One folder read the way the walk reads it: every entry, unless the folder
/// is above `MAX_AHEAD_ENTRIES` or the opening fails (then the walk reads it).
fn list(job: &Job, cancel: &AtomicBool) -> Read {
    let Ok((device, items)) = open(
        &job.directory,
        job.parent.as_ref(),
        job.follow_symlinks,
        cancel,
    ) else {
        return Read::Reread;
    };
    let mut entries = Vec::new();
    for item in items {
        if entries.len() == MAX_AHEAD_ENTRIES {
            return Read::Reread;
        }
        entries.push(item);
    }
    Read::Listed(device, entries)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
