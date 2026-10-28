//! What the pool shares with its channels: the signal that a lease ended
//! (waiting transfers wake up), the count of running leases, and the
//! throughput window that decides about one more SSH connection. File data
//! reported by the readers and writers fills the window; when it ends the
//! rule of `link_rules::Growth` answers, and an extra connection is set up
//! on a thread of its own (channel_pool.rs `start_growth`), never in the
//! data path.
use super::channel_pool::ChannelPool;
use super::link_rules::{blocked, Growth, Step, WINDOW};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

struct Window {
    started: Instant,
    bytes: u64,
    growth: Growth,
    /// A connection is being set up.
    growing: bool,
}

pub(super) struct PoolHub {
    /// Notified whenever a lease ends.
    pub(super) released: Condvar,
    active: AtomicUsize,
    links: AtomicUsize,
    /// Block key of the account (`link_rules::account_key`); `None`: this
    /// backend opens no extra connections.
    account: Option<String>,
    window: Mutex<Window>,
    pool: OnceLock<Weak<ChannelPool>>,
}

impl PoolHub {
    pub(super) fn new(account: Option<String>) -> Self {
        Self {
            released: Condvar::new(),
            active: AtomicUsize::new(0),
            links: AtomicUsize::new(1),
            account,
            window: Mutex::new(Window {
                started: Instant::now(),
                bytes: 0,
                growth: Growth::new(),
                growing: false,
            }),
            pool: OnceLock::new(),
        }
    }

    /// Tells the hub which pool to ask for a connection.
    pub(super) fn bind(&self, pool: Weak<ChannelPool>) {
        let _ = self.pool.set(pool);
    }

    pub(super) fn lease_started(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
    }

    pub(super) fn lease_ended(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.released.notify_all();
    }

    /// The SSH connections open now, the main one included.
    pub(super) fn set_links(&self, links: usize) {
        self.links.store(links.max(1), Ordering::Release);
    }

    /// `bytes` of file data moved. At the end of a window the growth rule
    /// decides whether one more connection is set up.
    pub(super) fn note(&self, bytes: u64) {
        let Some(account) = self.account.as_deref() else {
            return;
        };
        let now = Instant::now();
        let step = {
            let mut window = lock(&self.window);
            window.bytes = window.bytes.saturating_add(bytes);
            let elapsed = now.saturating_duration_since(window.started);
            if elapsed < WINDOW {
                return;
            }
            let started = window.started;
            let moved = std::mem::take(&mut window.bytes);
            window.started = now;
            if elapsed > WINDOW * 3 {
                // The transfer paused: nothing in this window says how fast it is.
                window.growth.idle();
                return;
            }
            let rate = moved as f64 / elapsed.as_secs_f64();
            let wants = self.demand() && !window.growing && blocked(account).is_none();
            let step = window.growth.window(started, rate, wants);
            if step == Step::Open {
                window.growing = true;
            }
            step
        };
        if step == Step::Open {
            match self.pool.get().and_then(Weak::upgrade) {
                Some(pool) => ChannelPool::start_growth(&pool),
                None => self.attempt_cancelled(),
            }
        }
    }

    /// More streams run than the open connections carry (two each).
    fn demand(&self) -> bool {
        self.active.load(Ordering::Acquire) >= 2 * self.links.load(Ordering::Acquire)
    }

    /// The connection is up after a setup of `setup`: its effect is judged
    /// once it carried data (`carried`), at the latest after `SETTLE`.
    pub(super) fn attempt_installed(&self, setup: Duration) {
        let mut window = lock(&self.window);
        window.growth.installed(Instant::now(), setup);
        window.growing = false;
    }

    /// The newest extra connection moved its first file data.
    pub(super) fn carried(&self) {
        lock(&self.window).growth.carried(Instant::now());
    }

    /// Setting it up failed: no more attempts.
    pub(super) fn attempt_failed(&self) {
        let mut window = lock(&self.window);
        window.growth.failed();
        window.growing = false;
    }

    /// The attempt could not start (blocked, another login running, no
    /// memory): a later window may try again.
    pub(super) fn attempt_cancelled(&self) {
        let mut window = lock(&self.window);
        window.growth.cancelled();
        window.growing = false;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
