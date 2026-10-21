//! FTP control connections for transfers. FTP moves one file per control
//! connection, so transfers run in parallel only on connections of their
//! own; the connection opened at connect time stays reserved for browsing
//! and metadata (plan K24). A transfer takes an idle pooled connection or
//! logs in a new one. The server's limit is learned from its first refusal
//! (421 at the greeting or 421/530 at the login): the browsing connection
//! already logged in with the same account, so the refusal is a connection
//! limit, not a wrong password (plan K25, docs/refs/ftp-pool.md). At the
//! limit a transfer gets congestion instead of waiting: a transfer that
//! waits while it holds a connection itself (a copy on the same server)
//! could wait forever, which is why rclone warns that its FTP connection cap
//! "is very likely to cause deadlocks". A server that accepts only one
//! connection serves transfers on the browsing connection, as before.
//!
//! Ranges of one large file (E13, answer to the security question S12) never
//! log in: they run on connections that are logged in and free already, and
//! only when at least two are (`ranges_ready`). A 421 or 530 to a range ends
//! the file as one stream and keeps ranges off for fifteen minutes
//! (`refuse_ranges`).
use super::connection::refusal_code;
use super::io_adapters::{FtpConnection, FtpReconnect};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// An idle connection this long belongs to a finished burst and is closed;
/// rclone empties its FTP pool after the same time (`--ftp-idle-timeout`).
const IDLE_RETIRE: Duration = Duration::from_secs(60);
/// After the server answered a range with 421 or 530 no range is tried for
/// this long (like the engine's memory of a connection that cannot read
/// ranges).
const RANGES_OFF_FOR: Duration = Duration::from_secs(15 * 60);
/// Ranges need this many logged-in free connections: one alone would be a
/// single stream with extra steps.
const RANGE_CONNECTIONS: usize = 2;

pub(super) struct FtpPool {
    primary: Arc<FtpConnection>,
    connect: FtpReconnect,
    state: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    idle: Vec<(Arc<FtpConnection>, Instant)>,
    busy: usize,
    opening: usize,
    /// Connections the server accepts from us, the browsing one included.
    limit: Option<usize>,
    /// No ranges until then (a 421 or 530 answered one).
    no_ranges_until: Option<Instant>,
}

/// Whether ranges may run: `idle` logged-in free connections, no recent
/// refusal, and a server limit that leaves room for two transfers.
pub(super) fn ranges_possible(
    idle: usize,
    no_ranges_until: Option<Instant>,
    capacity: Option<usize>,
    now: Instant,
) -> bool {
    idle >= RANGE_CONNECTIONS
        && no_ranges_until.is_none_or(|until| until <= now)
        && capacity.is_none_or(|capacity| capacity >= RANGE_CONNECTIONS)
}

/// A transfer's control connection; a pooled one goes back on drop.
pub(super) struct FtpLease {
    pool: Arc<FtpPool>,
    connection: Arc<FtpConnection>,
    pooled: bool,
}

impl FtpLease {
    pub(super) fn connection(&self) -> &Arc<FtpConnection> {
        &self.connection
    }
}

impl Drop for FtpLease {
    fn drop(&mut self) {
        if self.pooled {
            self.pool.give_back(self.connection.clone());
        }
    }
}

/// Transfer connections allowed next to the browsing one.
fn capacity(limit: Option<usize>) -> Option<usize> {
    limit.map(|limit| limit.saturating_sub(1))
}

fn lock(state: &Mutex<PoolState>) -> MutexGuard<'_, PoolState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FtpPool {
    pub(super) fn new(primary: Arc<FtpConnection>, connect: FtpReconnect) -> Arc<Self> {
        Arc::new(Self {
            primary,
            connect,
            state: Mutex::new(PoolState::default()),
        })
    }

    /// The browsing connection.
    pub(super) fn primary(&self) -> &Arc<FtpConnection> {
        &self.primary
    }

    /// Transfer connections the server allows next to the browsing one;
    /// `None` until it has refused one.
    pub(super) fn transfer_capacity(&self) -> Option<usize> {
        capacity(lock(&self.state).limit)
    }

    /// Closes connections idle for longer than `IDLE_RETIRE` (their
    /// keepalive would hold them open forever).
    pub(super) fn retire_idle(&self) {
        let now = Instant::now();
        lock(&self.state)
            .idle
            .retain(|(_, since)| now.saturating_duration_since(*since) < IDLE_RETIRE);
    }

    /// Whether a file can be read in ranges now: no login is needed for it.
    pub(super) fn ranges_ready(&self) -> bool {
        self.retire_idle();
        let state = lock(&self.state);
        ranges_possible(
            state.idle.len(),
            state.no_ranges_until,
            capacity(state.limit),
            Instant::now(),
        )
    }

    /// A logged-in free connection for one range; never a new login and
    /// never the browsing connection.
    pub(super) fn lease_idle(self: &Arc<Self>) -> Option<FtpLease> {
        self.retire_idle();
        let mut state = lock(&self.state);
        let (connection, _) = state.idle.pop()?;
        state.busy += 1;
        drop(state);
        Some(self.leased(connection, true))
    }

    /// The server turned a range down (421 or 530): no ranges for a while,
    /// and the connections it accepts are no more than are open now.
    pub(super) fn refuse_ranges(&self) {
        let mut state = lock(&self.state);
        state.no_ranges_until = Some(Instant::now() + RANGES_OFF_FOR);
        let accepted = 1 + state.busy + state.idle.len();
        state.limit = Some(state.limit.map_or(accepted, |known| known.min(accepted)));
    }

    /// A control connection for one transfer.
    pub(super) fn lease(self: &Arc<Self>) -> io::Result<FtpLease> {
        self.retire_idle();
        loop {
            {
                let mut state = lock(&self.state);
                if let Some((connection, _)) = state.idle.pop() {
                    state.busy += 1;
                    return Ok(self.leased(connection, true));
                }
                match capacity(state.limit) {
                    Some(0) => return Ok(self.leased(self.primary.clone(), false)),
                    Some(capacity) if state.busy + state.opening >= capacity => {
                        return Err(pool_full())
                    }
                    _ => state.opening += 1,
                }
            }
            let connected = (self.connect)()
                .and_then(|stream| FtpConnection::new(stream, self.connect.clone()));
            let mut state = lock(&self.state);
            state.opening = state.opening.saturating_sub(1);
            match connected {
                Ok(connection) => {
                    connection.inherit_server_facts(&self.primary);
                    state.busy += 1;
                    return Ok(self.leased(connection, true));
                }
                Err(error) if refusal_code(&error).is_some() => {
                    let accepted = 1 + state.busy + state.idle.len();
                    state.limit = Some(state.limit.map_or(accepted, |known| known.min(accepted)));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn leased(self: &Arc<Self>, connection: Arc<FtpConnection>, pooled: bool) -> FtpLease {
        FtpLease {
            pool: self.clone(),
            connection,
            pooled,
        }
    }

    fn give_back(&self, connection: Arc<FtpConnection>) {
        let mut state = lock(&self.state);
        state.busy = state.busy.saturating_sub(1);
        let room =
            capacity(state.limit).is_none_or(|capacity| state.busy + state.idle.len() < capacity);
        if room {
            state.idle.push((connection, Instant::now()));
        }
    }
}

pub(super) fn pool_full() -> io::Error {
    crate::vfs::congestion_error(
        "Der FTP-Server nimmt keine weitere Verbindung an; die Übertragung wartet auf eine freie",
        None,
    )
}

/// A transfer stream that keeps its connection until it is dropped. The
/// stream is dropped first (it hands its control stream back), then the
/// lease returns the connection to the pool.
pub(super) struct Leased<S> {
    stream: S,
    _lease: FtpLease,
}

impl<S> Leased<S> {
    pub(super) fn new(stream: S, lease: FtpLease) -> Self {
        Self {
            stream,
            _lease: lease,
        }
    }
}

impl<S: Read> Read for Leased<S> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buffer)
    }
}

impl<S: Write> Write for Leased<S> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.stream.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
