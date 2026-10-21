//! Extra SFTP channels for file transfers. One channel is one `sftp-server`
//! process with its own SSH window (OpenSSH grants 2 MiB per session
//! channel), so files spread over several channels move in parallel on the
//! server and uploads get one window each. Listing and metadata stay on the
//! main session.
//!
//! A transfer leases the least busy channel; while every channel is busy a
//! new one is opened on the main connection, as rclone opens one SFTP session
//! per concurrent transfer. Idle channels close after a minute, except the
//! last one used. The server's limit is learned from its first refusal
//! (OpenSSH: `MaxSessions`, default 10, answered with "open failed"): the
//! pool then keeps two channels free for the agent's exec channel and the
//! short-lived posix-rename channel (plan K25) and shares its channels
//! instead, since SFTP multiplexes requests of many files on one channel, at
//! most `min(open-handles, 64)` handles each (E13-S13); beyond that a range
//! waits for a lease to end. A range never fails for lack of a channel: it
//! shares, waits or, with no channel at all, uses the main session
//! (`lease_range`). A single stream without a channel uses the main session.
//!
//! One SSH connection carries one TCP stream, and on a long fat link its
//! kernel buffer caps the rate however many channels it has. So the pool may
//! add extra SSH connections (security answer E13-S15, extra_link.rs), each
//! with one channel, one at a time and only while the last one raised the
//! throughput by 10 % (link_rules.rs `Growth`, pool_growth.rs).
use super::backend::SftpBackend;
use super::connection::{SftpConnection, SftpGeneration};
use super::exec::ChannelOpen;
pub(super) use super::link_rules::pool_usable;
use super::link_rules::{account_key, limit_after_refusal, seat, Candidate, ExtraLogin, Seat};
#[cfg(test)]
pub(super) use super::link_rules::{learned_limit, pick, Pick};
#[cfg(test)]
pub(super) use super::pool_channel::offered;
pub(super) use super::pool_channel::{note_failure, open_failed, ChannelLease, PoolChannel};
use super::pool_channel::{setup, Owner};
use super::pool_extra::ExtraSupport;
use super::pool_growth::PoolHub;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

/// A channel unused this long belongs to a finished burst; rclone empties
/// its SFTP pool after the same idle time (`--sftp-idle-timeout` 1m0s). Extra
/// connections close after it too.
const IDLE_RETIRE: Duration = Duration::from_secs(60);
/// A waiting transfer looks again this often, also without a wake-up.
const WAIT_SLICE: Duration = Duration::from_millis(200);
/// A range waits this long for a channel before it uses the main session.
const RANGE_WAIT: Duration = Duration::from_secs(30);

pub(super) struct ChannelPool {
    pub(super) state: Mutex<PoolState>,
    /// The server refused `copy-data` once: its request policy holds for
    /// every channel, so later copies stream without asking again.
    pub(super) copy_refused: AtomicBool,
    /// Channels the server accepted whose setup is not finished: they count
    /// against its limit but are not in `state.channels` yet.
    pub(super) setting_up: AtomicUsize,
    /// Extra connections open (for the log lines).
    pub(super) open_links: Arc<AtomicUsize>,
    pub(super) hub: Arc<PoolHub>,
    pub(super) support: Option<ExtraSupport>,
    pub(super) watching: AtomicBool,
}

#[derive(Default)]
pub(super) struct PoolState {
    pub(super) channels: Vec<Arc<PoolChannel>>,
    opening: usize,
    /// Channels the main connection may hold; extra connections add theirs.
    limit: Option<usize>,
    /// Setup of a channel failed on this generation: the main session serves.
    disabled: Option<Weak<SftpGeneration>>,
}

enum Choice {
    Use(ChannelLease),
    Open,
    Wait,
    MainSession,
}

impl PoolState {
    fn prune(&mut self, generation: &Arc<SftpGeneration>, now: Instant) {
        for channel in &self.channels {
            if !channel.usable_on(generation) {
                channel.note_close("Hauptverbindung erneuert");
            }
        }
        self.channels
            .retain(|channel| channel.usable_on(generation));
        self.retire_idle(now);
    }

    fn retire_idle(&mut self, now: Instant) {
        // The most recently used idle channel of the main connection stays
        // open, so a single later file does not pay the channel setup (three
        // round trips) again. Extra connections keep none: a minute without
        // a transfer closes them.
        let warm = self
            .channels
            .iter()
            .filter(|channel| !channel.is_extra() && channel.load() == 0)
            .min_by_key(|channel| channel.idle_for(now))
            .cloned();
        for channel in &self.channels {
            if channel.load() == 0 && channel.idle_for(now) >= IDLE_RETIRE {
                channel.note_close("Leerlauf 60 s");
            }
        }
        self.channels.retain(|channel| {
            channel.load() > 0
                || channel.idle_for(now) < IDLE_RETIRE
                || warm.as_ref().is_some_and(|warm| Arc::ptr_eq(warm, channel))
        });
    }

    fn disabled_for(&self, generation: &Arc<SftpGeneration>) -> bool {
        self.disabled
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|disabled| Arc::ptr_eq(&disabled, generation))
    }

    pub(super) fn extra_channels(&self) -> usize {
        self.channels
            .iter()
            .filter(|channel| channel.is_extra())
            .count()
    }

    fn main_channels(&self) -> usize {
        self.channels.len() - self.extra_channels()
    }

    /// Channels in all: the main connection's limit plus one per extra
    /// connection; `None` while the main limit is not known.
    fn total_limit(&self) -> Option<usize> {
        self.limit.map(|limit| limit + self.extra_channels())
    }

    fn choose(&mut self) -> Choice {
        let seats: Vec<Candidate> = self
            .channels
            .iter()
            .map(|channel| Candidate {
                load: channel.load(),
                cap: channel.handle_cap(),
            })
            .collect();
        match seat(&seats, self.opening, self.total_limit()) {
            Seat::Use(index) => match self.channels.get(index) {
                Some(channel) => Choice::Use(ChannelLease::new(channel.clone())),
                None => Choice::MainSession,
            },
            Seat::Open => {
                self.opening += 1;
                Choice::Open
            }
            Seat::Wait => Choice::Wait,
            Seat::MainSession => Choice::MainSession,
        }
    }

    /// The server refused a channel of the main connection while it held
    /// the ready ones and `in_setup` more that it had accepted.
    fn refused(&mut self, in_setup: usize) {
        let limit = limit_after_refusal(self.limit, self.main_channels(), in_setup);
        self.limit = Some(limit);
        self.shrink_main(limit);
    }

    /// Busy channels of the main connection first: the idle ones beyond the
    /// limit close now, a busy one after its last lease (it gets no new
    /// ones). Extra connections stay.
    fn shrink_main(&mut self, limit: usize) {
        let (extra, mut main): (Vec<_>, Vec<_>) = self
            .channels
            .drain(..)
            .partition(|channel| channel.is_extra());
        main.sort_by_key(|channel| std::cmp::Reverse(channel.load()));
        main.truncate(limit);
        main.extend(extra);
        self.channels = main;
    }
}

enum Opened {
    Ready(PoolChannel),
    Refused,
    Unavailable(Arc<SftpGeneration>),
}

/// Counts a channel the server accepted while its setup runs.
struct SettingUp<'a>(&'a AtomicUsize);

impl<'a> SettingUp<'a> {
    fn enter(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::AcqRel);
        Self(counter)
    }
}

impl Drop for SettingUp<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    /// A whole file as one stream: the main session is a fine fallback.
    Single,
    /// One range of a file: it shares, waits or, with no channel at all,
    /// the caller uses the main session.
    Range,
}

impl ChannelPool {
    /// The pool of a backend. `login` is `Some` only when the main login
    /// allows extra connections (`link_rules::plan`).
    pub(super) fn create(
        connection: &Arc<SftpConnection>,
        host: &str,
        port: u16,
        user: &str,
        login: Option<ExtraLogin>,
    ) -> Arc<Self> {
        let account = login.as_ref().map(|_| account_key(host, port, user));
        let support = login.map(|login| ExtraSupport::new(connection, host, port, user, login));
        let pool = Arc::new(Self {
            state: Mutex::new(PoolState::default()),
            copy_refused: AtomicBool::new(false),
            setting_up: AtomicUsize::new(0),
            open_links: Arc::new(AtomicUsize::new(0)),
            hub: Arc::new(PoolHub::new(account)),
            support,
            watching: AtomicBool::new(false),
        });
        pool.hub.bind(Arc::downgrade(&pool));
        pool
    }

    pub(super) fn copy_data_refused(&self) -> bool {
        self.copy_refused.load(Ordering::Acquire)
    }

    pub(super) fn refuse_copy_data(&self) {
        self.copy_refused.store(true, Ordering::Release);
    }

    /// Closes channels idle past `IDLE_RETIRE` (all but the warm one)
    /// without waiting for the next transfer.
    pub(super) fn retire_idle(&self) {
        let mut state = lock(&self.state);
        state.retire_idle(Instant::now());
        self.hub.set_links(1 + state.extra_channels());
    }

    /// Whether `lease` can hand out a channel for the current connection
    /// generation (without opening one).
    pub(super) fn available(&self, backend: &SftpBackend) -> bool {
        let Ok(generation) = backend.connection.current() else {
            return false;
        };
        let mut state = lock(&self.state);
        state.prune(&generation, Instant::now());
        pool_usable(
            state.disabled_for(&generation),
            state.channels.len(),
            state.limit,
        )
    }

    /// The open-handle limit of the first pool channel that reported one.
    pub(super) fn known_open_handles(&self) -> Option<u64> {
        lock(&self.state)
            .channels
            .iter()
            .find_map(|channel| channel.open_handles())
    }

    /// Transfers the connections carry at once (channels × handles each),
    /// once the main limit is known: the flow control may not raise the
    /// parallel files and ranges beyond it, where only refusals would
    /// follow. `None`: nothing is known to bound them.
    pub(super) fn capacity(&self) -> Option<usize> {
        let state = lock(&self.state);
        let channels = state.total_limit()?;
        if channels == 0 {
            return None;
        }
        let handles = state
            .channels
            .iter()
            .map(|channel| channel.handle_cap())
            .min()
            .unwrap_or(super::batch_ops::OPEN_HANDLES);
        Some(channels.saturating_mul(handles))
    }

    /// A channel for one file transfer; `None`: use the main session.
    pub(super) fn lease(&self, backend: &SftpBackend) -> io::Result<Option<ChannelLease>> {
        self.acquire(backend, Need::Single)
    }

    /// A channel for one range of a file. It never fails for lack of a free
    /// channel: it shares one (up to its handle limit) or waits for a lease
    /// to end; `None` only when the pool can hold no channel at all (or
    /// none became free within `RANGE_WAIT`): the caller uses the main
    /// session.
    pub(super) fn lease_range(&self, backend: &SftpBackend) -> io::Result<Option<ChannelLease>> {
        self.acquire(backend, Need::Range)
    }

    fn acquire(&self, backend: &SftpBackend, need: Need) -> io::Result<Option<ChannelLease>> {
        let give_up = Instant::now() + RANGE_WAIT;
        loop {
            let generation = backend.connection.current()?;
            let mut state = lock(&self.state);
            state.prune(&generation, Instant::now());
            self.hub.set_links(1 + state.extra_channels());
            if state.disabled_for(&generation) && state.channels.is_empty() {
                return Ok(None);
            }
            match state.choose() {
                Choice::Use(lease) => return Ok(Some(lease)),
                Choice::MainSession => return Ok(None),
                Choice::Wait => {
                    if need == Need::Single || Instant::now() >= give_up {
                        return Ok(None);
                    }
                    let (guard, _) = self
                        .hub
                        .released
                        .wait_timeout(state, WAIT_SLICE)
                        .unwrap_or_else(PoisonError::into_inner);
                    drop(guard);
                    continue;
                }
                Choice::Open => {}
            }
            drop(state);
            let opened = self.open_channel(backend);
            let mut state = lock(&self.state);
            state.opening = state.opening.saturating_sub(1);
            match opened {
                Ok(Opened::Ready(channel)) => {
                    let channel = Arc::new(channel);
                    let lease = ChannelLease::new(channel.clone());
                    state.channels.push(channel);
                    return Ok(Some(lease));
                }
                Ok(Opened::Refused) => state.refused(self.setting_up.load(Ordering::Acquire)),
                Ok(Opened::Unavailable(generation)) => {
                    let held = state.main_channels() + self.setting_up.load(Ordering::Acquire);
                    if held == 0 {
                        state.disabled = Some(Arc::downgrade(&generation));
                        return Ok(None);
                    }
                    // Other channels work: this one only did not fit now.
                    let limit = state.limit.map_or(held, |known| known.min(held));
                    state.limit = Some(limit);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn open_channel(&self, backend: &SftpBackend) -> io::Result<Opened> {
        let (generation, channel) = match backend.try_open_session_channel()? {
            ChannelOpen::Opened(generation, channel) => (generation, channel),
            ChannelOpen::Refused(_) => return Ok(Opened::Refused),
        };
        let _counted = SettingUp::enter(&self.setting_up);
        match backend.rt.block_on(setup(channel)) {
            Ok(setup) => Ok(Opened::Ready(PoolChannel::new(
                setup,
                Owner::Main(generation),
                self.hub.clone(),
            ))),
            // A lost connection fails the transfer (the next call reconnects);
            // any other setup failure only turns the pool off for this session.
            Err(error) if generation.session().is_closed() => {
                backend.connection.mark_stale(&generation);
                Err(error)
            }
            Err(_) => Ok(Opened::Unavailable(generation)),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
