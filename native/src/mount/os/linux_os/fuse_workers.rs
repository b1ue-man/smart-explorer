//! Threads of the FUSE host (DRV-M4, decision C2 "Threading").
//!
//! `fuser` reads requests on two loops (`n_threads = 2`, `clone_fd`). A loop
//! must never wait for a backend: every request that can reach the engine
//! goes with its `Reply` to this elastic pool, and a worker answers from its
//! own thread. There is no fixed upper bound: the kernel bounds background
//! requests (`max_background`, 16 in `fuser`) and synchronous ones by their
//! blocked callers. Two workers stay; a surplus worker ends once the queue
//! stays empty for `LINGER`. A job that panics drops its reply, which
//! `fuser` answers with `EIO`, and asks the host to stop (fail-stop, like the
//! Dokany callback guards).
//!
//! The metadata refresh keeps the cache of a running drive current, with
//! the cadence of the Windows host (`metadata_refresh.rs`, 20 s).
use std::collections::VecDeque;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::mount::MountFilesystem;

/// Workers that stay while the queue is empty.
const CORE_WORKERS: usize = 2;
/// How long a surplus worker waits for more work before it ends.
const LINGER: Duration = Duration::from_secs(2);
const REFRESH_INTERVAL: Duration = Duration::from_secs(20);
/// How long the end of a drive waits for a refresh that is inside a backend call.
const REFRESH_JOIN_GRACE: Duration = Duration::from_secs(5);

pub(crate) type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone)]
pub(crate) struct WorkerPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    state: Mutex<PoolState>,
    work: Condvar,
    idle: Condvar,
    on_panic: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct PoolState {
    jobs: VecDeque<Job>,
    workers: usize,
    waiting: usize,
    running: usize,
    closed: bool,
}

impl WorkerPool {
    /// `on_panic` runs after a job panicked.
    pub(crate) fn new(on_panic: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                state: Mutex::new(PoolState::default()),
                work: Condvar::new(),
                idle: Condvar::new(),
                on_panic: Box::new(on_panic),
            }),
        }
    }

    /// Queues `job`; after `close` the job is dropped, so its reply answers
    /// `EIO`. Without any worker and without a new thread it runs here.
    pub(crate) fn submit(&self, job: Job) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        if state.closed {
            return;
        }
        state.jobs.push_back(job);
        if state.waiting > state.jobs.len().saturating_sub(1) {
            self.inner.work.notify_one();
            return;
        }
        state.workers += 1;
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("fuse-worker".into())
            .spawn(move || work(inner));
        if spawned.is_err() {
            state.workers -= 1;
            if state.workers == 0 {
                let job = state.jobs.pop_back();
                drop(state);
                if let Some(job) = job {
                    self.inner.run(job);
                }
            }
        }
    }

    /// Waits until no job is queued or running; `false` after `timeout`.
    pub(crate) fn drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let Ok(mut state) = self.inner.state.lock() else {
            return false;
        };
        while !state.jobs.is_empty() || state.running > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = match self.inner.idle.wait_timeout(state, left) {
                Ok((state, _)) => state,
                Err(_) => return false,
            };
        }
        true
    }

    /// No new job is taken; queued jobs are dropped (their replies answer
    /// `EIO`) and idle workers end.
    pub(crate) fn close(&self) {
        let jobs = match self.inner.state.lock() {
            Ok(mut state) => {
                state.closed = true;
                std::mem::take(&mut state.jobs)
            }
            Err(_) => return,
        };
        drop(jobs);
        self.inner.work.notify_all();
        self.inner.idle.notify_all();
    }
}

impl PoolInner {
    fn run(&self, job: Job) {
        if catch_unwind(AssertUnwindSafe(job)).is_err() {
            (self.on_panic)();
        }
    }
}

fn work(inner: Arc<PoolInner>) {
    loop {
        let job = {
            let Ok(mut state) = inner.state.lock() else {
                return;
            };
            loop {
                if let Some(job) = state.jobs.pop_front() {
                    state.running += 1;
                    break job;
                }
                if state.closed {
                    state.workers -= 1;
                    return;
                }
                state.waiting += 1;
                let surplus = state.workers > CORE_WORKERS;
                let waited = if surplus {
                    inner
                        .work
                        .wait_timeout(state, LINGER)
                        .ok()
                        .map(|(state, timeout)| (state, timeout.timed_out()))
                } else {
                    inner.work.wait(state).ok().map(|state| (state, false))
                };
                let Some((next, timed_out)) = waited else {
                    return;
                };
                state = next;
                state.waiting -= 1;
                if timed_out && state.jobs.is_empty() && state.workers > CORE_WORKERS {
                    state.workers -= 1;
                    return;
                }
            }
        };
        inner.run(job);
        if let Ok(mut state) = inner.state.lock() {
            state.running -= 1;
            if state.jobs.is_empty() && state.running == 0 {
                inner.idle.notify_all();
            }
        }
    }
}

/// The background metadata refresh of a running drive.
pub(crate) struct RefreshWorker {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl RefreshWorker {
    pub(crate) fn start(filesystem: Arc<dyn MountFilesystem>) -> io::Result<Self> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("fuse-metadata-refresh".into())
            .spawn(move || refresh(filesystem, worker_stop))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn request_stop(&self) {
        let (state, wake) = &*self.stop;
        if let Ok(mut stopped) = state.lock() {
            *stopped = true;
            wake.notify_all();
        }
    }

    /// Asks the refresh to stop and waits for it at most `REFRESH_JOIN_GRACE`.
    /// A refresh blocked in a backend call is left behind: the host process
    /// ends right after, and a drive that cannot be stopped because its
    /// backend hangs would keep its worker and its mount alive.
    pub(crate) fn join(&mut self) {
        self.request_stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now() + REFRESH_JOIN_GRACE;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

impl Drop for RefreshWorker {
    fn drop(&mut self) {
        self.join();
    }
}

fn refresh(filesystem: Arc<dyn MountFilesystem>, stop: Arc<(Mutex<bool>, Condvar)>) {
    let mut next_refresh = Instant::now() + REFRESH_INTERVAL;
    loop {
        if stopped(&stop) {
            return;
        }
        let _ = filesystem.maintain_cache();
        if Instant::now() >= next_refresh {
            let _ = filesystem.refresh_metadata_while(&|| stopped(&stop));
            next_refresh = Instant::now() + REFRESH_INTERVAL;
        }
        let progress = filesystem
            .preload_metadata_while(&|| stopped(&stop))
            .unwrap_or_default();
        if progress.loaded > 0 || progress.pending {
            continue;
        }
        let delay = next_refresh.saturating_duration_since(Instant::now());
        let Ok(guard) = stop.0.lock() else {
            return;
        };
        if *guard {
            return;
        }
        match stop.1.wait_timeout(guard, delay) {
            Ok((guard, _)) if !*guard => {}
            _ => return,
        }
    }
}

fn stopped(stop: &(Mutex<bool>, Condvar)) -> bool {
    stop.0.lock().map_or(true, |stopped| *stopped)
}
