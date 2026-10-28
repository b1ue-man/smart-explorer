//! Request threads of one backend session (Z-P3): a request runs on an idle
//! thread of the session instead of a thread started for it. A thread that
//! stays idle for `IDLE_EXIT` ends; at most `limit` requests run or wait at
//! once, as before. Starting a thread can still fail (`EAGAIN`): the request
//! is then refused exactly as before and never queued.
//!
//! Shutdown keeps the former guarantee: every accepted request runs to its
//! end and every thread is joined, so no request that may have dispatched a
//! remote mutation is ever detached.
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::super::request_workers::RequestWorkers;

/// An idle thread ends after this long without a request.
const IDLE_EXIT: Duration = Duration::from_secs(30);

type Job = Box<dyn FnOnce() -> io::Result<()> + Send + 'static>;

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    /// Requests accepted and not finished (queued or running).
    active: usize,
    /// Threads blocked waiting for a request. A queued request always has
    /// one of them (or a thread started for it): never more queued requests
    /// than waiting threads, so no request waits for another to finish.
    waiting: usize,
    /// Threads of the session that have not ended.
    threads: usize,
    closed: bool,
    errors: Vec<String>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    work: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The request threads of one session.
#[derive(Default)]
pub(super) struct RequestPool {
    shared: Arc<Shared>,
    /// Their handles: a thread that ended is joined (and a panic of the
    /// thread itself reported) like a request thread before.
    threads: RequestWorkers,
}

impl RequestPool {
    /// Whether one more request fits below `limit` accepted requests; the
    /// first failure of a finished request ends the session, as before.
    pub(super) fn has_capacity(&mut self, limit: usize) -> io::Result<bool> {
        // Joins the threads that ended while idle.
        self.threads.has_capacity(usize::MAX)?;
        let mut state = self.shared.lock();
        if !state.errors.is_empty() {
            let errors = std::mem::take(&mut state.errors);
            return Err(io::Error::other(errors.join("; ")));
        }
        Ok(state.active < limit)
    }

    /// Runs `job` on an idle thread, or on a new one. `Err`: no thread could
    /// be started; the job was not accepted.
    pub(super) fn submit(&mut self, job: Job) -> io::Result<()> {
        let mut state = self.shared.lock();
        if state.queue.len() >= state.waiting {
            // Started under the lock: it takes its first request only after
            // this one is queued.
            let shared = self.shared.clone();
            let thread = std::thread::Builder::new()
                .name("daemon-backend-request".into())
                .spawn(move || {
                    serve(&shared);
                    Ok(())
                })?;
            self.threads.push(thread);
            state.threads += 1;
        }
        state.active += 1;
        state.queue.push_back(job);
        drop(state);
        self.shared.work.notify_one();
        Ok(())
    }

    /// Runs every accepted request to its end and joins every thread.
    pub(super) fn shutdown(&mut self) -> io::Result<()> {
        self.shared.lock().closed = true;
        self.shared.work.notify_all();
        let joined = self.threads.shutdown();
        let mut errors = std::mem::take(&mut self.shared.lock().errors);
        if let Err(error) = joined {
            errors.push(error.to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
    }
}

/// One request thread: takes requests until it was idle for `IDLE_EXIT` or
/// the session closed with nothing left to run.
fn serve(shared: &Shared) {
    let mut state = shared.lock();
    loop {
        if let Some(job) = state.queue.pop_front() {
            drop(state);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            state = shared.lock();
            state.active = state.active.saturating_sub(1);
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => state.errors.push(error.to_string()),
                Err(payload) => state.errors.push(format!(
                    "daemon backend request worker panicked: {}",
                    panic_message(&*payload)
                )),
            }
            continue;
        }
        if state.closed {
            state.threads -= 1;
            return;
        }
        state.waiting += 1;
        let (next, timeout) = shared
            .work
            .wait_timeout(state, IDLE_EXIT)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state = next;
        state.waiting -= 1;
        if timeout.timed_out() && state.queue.is_empty() && !state.closed {
            state.threads -= 1;
            return;
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn speed_task_service_pool_reuses_threads_and_joins_every_request() {
        let mut pool = RequestPool::default();
        let ran = Arc::new(AtomicUsize::new(0));
        for _ in 0..50 {
            let ran = ran.clone();
            pool.submit(Box::new(move || {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }))
            .unwrap();
            // Sequential requests find the idle thread of the previous one.
            loop {
                let state = pool.shared.lock();
                if state.active == 0 && state.waiting == 1 {
                    break;
                }
                drop(state);
                std::thread::yield_now();
            }
        }
        assert_eq!(pool.shared.lock().threads, 1);
        assert!(pool.has_capacity(1).unwrap());
        pool.shutdown().unwrap();
        assert_eq!(ran.load(Ordering::SeqCst), 50);
    }

    #[test]
    fn speed_task_service_pool_counts_running_requests_and_reports_failures() {
        let mut pool = RequestPool::default();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        pool.submit(Box::new(move || {
            let _ = gate.recv();
            Err(io::Error::other("request failed"))
        }))
        .unwrap();
        assert!(!pool.has_capacity(1).unwrap());
        assert!(pool.has_capacity(2).unwrap());
        release.send(()).unwrap();
        let failed = loop {
            match pool.has_capacity(1) {
                Ok(_) => std::thread::yield_now(),
                Err(error) => break error,
            }
        };
        assert!(failed.to_string().contains("request failed"));
        pool.shutdown().unwrap();
    }

    #[test]
    fn speed_task_service_pool_shutdown_runs_queued_requests() {
        let mut pool = RequestPool::default();
        let ran = Arc::new(AtomicUsize::new(0));
        for _ in 0..8 {
            let ran = ran.clone();
            pool.submit(Box::new(move || {
                std::thread::sleep(Duration::from_millis(5));
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }))
            .unwrap();
        }
        pool.shutdown().unwrap();
        assert_eq!(ran.load(Ordering::SeqCst), 8);
        assert_eq!(pool.shared.lock().threads, 0);
    }
}
