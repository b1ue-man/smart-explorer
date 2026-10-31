//! A parallel tree walk without level barriers for walks next to the data
//! (agent `WalkHashed2`/`WalkHashed3`, Share host `hash_walk_v1`). Every
//! thread works through the folders it finds itself, newest first (depth
//! first, near the folder it just read). When another thread runs out of
//! work, the busy one first sends what it produced (`Output::flush`) and then
//! hands over its oldest folders. So every folder reaches the stream before
//! anything below it, no thread waits for a level to finish, and an idle
//! thread gets work as soon as one folder is done.
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How often an idle thread looks at the cancel flag.
const IDLE_SLICE: Duration = Duration::from_millis(100);

/// What one walking thread produced and has not sent yet.
pub(crate) trait Output {
    /// Sends everything produced so far, in the order it was produced.
    fn flush(&mut self) -> io::Result<()>;
}

/// Threads of a walk next to the data: the parallelism the host offers when
/// the walk starts (CPU affinity and cgroup quota included), at least one.
pub(crate) fn walk_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |threads| threads.get())
}

/// The folders one thread found and still has to read itself.
pub(crate) struct Local<J> {
    pending: Vec<J>,
}

impl<J> Local<J> {
    /// A folder below an entry this thread already put into its output.
    pub(crate) fn push(&mut self, job: J) {
        self.pending.push(job);
    }
}

struct State<J> {
    shared: Vec<J>,
    /// Threads holding a job (and the folders they found).
    busy: usize,
    idle: usize,
    error: Option<io::Error>,
    done: bool,
}

struct Pool<J> {
    state: Mutex<State<J>>,
    changed: Condvar,
    /// `State::idle`, readable without the lock after every folder.
    idle: AtomicUsize,
    /// Set by the first error: every thread stops after its current folder
    /// instead of reading its own pending folders to the end.
    stopped: AtomicBool,
}

impl<J> Pool<J> {
    fn lock(&self) -> MutexGuard<'_, State<J>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fail(&self, error: io::Error) {
        let mut state = self.lock();
        if state.error.is_none() {
            state.error = Some(error);
        }
        state.done = true;
        self.stopped.store(true, Ordering::Relaxed);
        drop(state);
        self.changed.notify_all();
    }

    /// The next job of an idle thread; `None` once the walk is over.
    fn take(&self, was_busy: &mut bool, cancel: &AtomicBool) -> Option<J> {
        let mut state = self.lock();
        if std::mem::take(was_busy) {
            state.busy -= 1;
        }
        loop {
            if state.done || state.error.is_some() {
                return None;
            }
            if cancel.load(Ordering::Relaxed) {
                drop(state);
                self.fail(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "tree walk canceled",
                ));
                return None;
            }
            if let Some(job) = state.shared.pop() {
                state.busy += 1;
                *was_busy = true;
                return Some(job);
            }
            if state.busy == 0 {
                state.done = true;
                drop(state);
                self.changed.notify_all();
                return None;
            }
            state.idle += 1;
            self.idle.store(state.idle, Ordering::Relaxed);
            state = match self.changed.wait_timeout(state, IDLE_SLICE) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
            state.idle -= 1;
            self.idle.store(state.idle, Ordering::Relaxed);
        }
    }

    /// Hands every pending folder but the newest to the idle threads.
    fn give(&self, pending: &mut Vec<J>) {
        let keep = pending.pop();
        let mut state = self.lock();
        state.shared.append(pending);
        drop(state);
        pending.extend(keep);
        self.changed.notify_all();
    }
}

/// Fails the walk when its thread ends by a panic, so no other thread waits
/// for this one's folders forever.
struct Enlisted<'p, J> {
    pool: &'p Pool<J>,
}

impl<J> Drop for Enlisted<'_, J> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.pool
                .fail(io::Error::other("a tree walk thread stopped unexpectedly"));
        }
    }
}

/// Walks from `roots` on `threads` threads. `work` reads one folder, puts its
/// entries into the thread's output and pushes the folders below it; the
/// first error (or a cancel) ends the walk and is returned; every thread then
/// stops after the folder it is reading. Each thread's output comes from
/// `output` and is flushed before the thread waits or ends.
pub(crate) fn run<J, O, M, W>(
    roots: Vec<J>,
    threads: usize,
    cancel: &AtomicBool,
    output: M,
    work: W,
) -> io::Result<()>
where
    J: Send,
    O: Output,
    M: Fn() -> O + Sync,
    W: Fn(J, &mut Local<J>, &mut O) -> io::Result<()> + Sync,
{
    let pool = Pool {
        state: Mutex::new(State {
            shared: roots,
            busy: 0,
            idle: 0,
            error: None,
            done: false,
        }),
        changed: Condvar::new(),
        idle: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
    };
    let worker = || {
        let _enlisted = Enlisted { pool: &pool };
        let mut out = output();
        let mut local = Local {
            pending: Vec::new(),
        };
        let mut busy = false;
        loop {
            // Another thread failed: its error ends the walk, so nothing this
            // thread still holds is read or sent.
            if pool.stopped.load(Ordering::Relaxed) {
                break;
            }
            let job = match local.pending.pop() {
                Some(job) => job,
                None => {
                    if let Err(error) = out.flush() {
                        pool.fail(error);
                        break;
                    }
                    match pool.take(&mut busy, cancel) {
                        Some(job) => job,
                        None => break,
                    }
                }
            };
            if let Err(error) = work(job, &mut local, &mut out) {
                pool.fail(error);
                break;
            }
            if local.pending.len() > 1 && pool.idle.load(Ordering::Relaxed) > 0 {
                if let Err(error) = out.flush() {
                    pool.fail(error);
                    break;
                }
                pool.give(&mut local.pending);
            }
        }
        if busy {
            pool.lock().busy -= 1;
            pool.changed.notify_all();
        }
    };
    let worker = &worker;
    std::thread::scope(|scope| {
        for index in 1..threads.max(1) {
            // A refused thread only means fewer helpers: the caller walks too.
            let _ = std::thread::Builder::new()
                .name(format!("tree-walk-{index}"))
                .spawn_scoped(scope, move || worker());
        }
        worker();
    });
    let state = pool
        .state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match state.error {
        Some(error) => Err(error),
        None if cancel.load(Ordering::Relaxed) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "tree walk canceled",
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "ordered_walk_tests.rs"]
mod tests;
