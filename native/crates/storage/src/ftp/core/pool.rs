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
//! Logins run on threads of their own (2026-10-29, suite run 2: `se cp` of
//! 5000 small files from vsftpd took 2 s or 12 s; the extra 10 s match one
//! login that reached the 10 s setup deadline while the other connections
//! had long finished their files, the bench with a warm pool never showed
//! it). A transfer that needs a new connection waits for
//! its own login or for a connection another transfer gives back, whichever
//! comes first; the listing of a folder starts spare logins for its SIZE and
//! MDTM probes without waiting for them (`open_spares`).
//!
//! Ranges of one large file (G3-11, decided 2026-10-28 by the main agent on
//! the user's instruction "maximal möglicher Datentransferspeed", superseding
//! the security answer S12): a range takes a logged-in free connection or
//! logs in one more with the same credentials and the same TLS policy as
//! every pool connection (`lease_range`), like `lftp pget -n`; how many run
//! at once is the flow control's measured window, and the server's own limit
//! (421/530 at a login) caps them as for whole files. Never the browsing
//! connection. A 421 or 530 to REST/RETR of a range ends the file as one
//! stream and keeps ranges off for fifteen minutes (`refuse_ranges`).
use super::connection::refusal_code;
use super::io_adapters::{FtpConnection, FtpReconnect};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// An idle connection this long belongs to a finished burst and is closed;
/// rclone empties its FTP pool after the same time (`--ftp-idle-timeout`).
const IDLE_RETIRE: Duration = Duration::from_secs(60);
/// After the server answered a range with 421 or 530 no range is tried for
/// this long (like the engine's memory of a connection that cannot read
/// ranges).
const RANGES_OFF_FOR: Duration = Duration::from_secs(15 * 60);
/// Ranges need room for this many transfer connections: one alone would be
/// a single stream with extra steps.
const RANGE_CONNECTIONS: usize = 2;

pub(super) struct FtpPool {
    primary: Arc<FtpConnection>,
    connect: FtpReconnect,
    state: Mutex<PoolState>,
    /// A connection came back or a login ended (`lease` waits for either).
    changed: Condvar,
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
    /// Logins a `lease` waits for, by number; `None` while running. A login
    /// whose lease took a returned connection meanwhile is removed here, and
    /// its connection goes to `idle` when it is ready.
    logins: HashMap<u64, Option<io::Result<Arc<FtpConnection>>>>,
    next_login: u64,
}

/// Whether ranges may run: no recent refusal, and a server limit (when
/// known) that leaves room for two transfer connections. Free connections
/// are not needed any more: a range logs in one (G3-11).
pub(super) fn ranges_possible(
    no_ranges_until: Option<Instant>,
    capacity: Option<usize>,
    now: Instant,
) -> bool {
    no_ranges_until.is_none_or(|until| until <= now)
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
            changed: Condvar::new(),
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

    /// Whether a file can be read in ranges now.
    pub(super) fn ranges_ready(&self) -> bool {
        self.retire_idle();
        let state = lock(&self.state);
        ranges_possible(state.no_ranges_until, capacity(state.limit), Instant::now())
    }

    /// A connection for one range: a logged-in free one, else a new login
    /// (same credentials and TLS policy, `connect`), never the browsing
    /// connection. `None` once the server is known to allow fewer than two
    /// transfer connections (the file then goes as one stream); at a limit
    /// with room for ranges the range gets congestion and the flow control
    /// holds back.
    pub(super) fn lease_range(self: &Arc<Self>) -> io::Result<Option<FtpLease>> {
        if !self.room_for_ranges() {
            return Ok(None);
        }
        match self.lease() {
            Ok(lease) if lease.pooled => Ok(Some(lease)),
            // Only the browsing connection is left: no range on it.
            Ok(_) => Ok(None),
            // The login for the range taught a limit without room for ranges.
            Err(_) if !self.room_for_ranges() => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn room_for_ranges(&self) -> bool {
        capacity(lock(&self.state).limit).is_none_or(|capacity| capacity >= RANGE_CONNECTIONS)
    }

    /// A logged-in free connection (metadata of parallel transfers); never
    /// a new login and never the browsing connection.
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

    /// A control connection for one transfer: a free one, else a new login.
    /// The login runs on a thread of its own while this call also takes a
    /// connection another transfer gives back meanwhile, whichever comes
    /// first: a login that hangs (a server that greets late, a SYN lost at
    /// its full accept queue, up to the setup deadline) then holds up no
    /// file while other connections turn over. A login nobody waits for any
    /// more joins the free connections when it is ready.
    pub(super) fn lease(self: &Arc<Self>) -> io::Result<FtpLease> {
        self.retire_idle();
        let mut state = lock(&self.state);
        let mut login: Option<u64> = None;
        loop {
            if let Some((connection, _)) = state.idle.pop() {
                // The own login goes on for the next transfer; one that is
                // already in joins the free connections.
                if let Some(Some(Ok(ready))) = login.and_then(|id| state.logins.remove(&id)) {
                    state.idle.push((ready, Instant::now()));
                }
                state.busy += 1;
                return Ok(self.leased(connection, true));
            }
            if let Some(id) = login {
                let ended = matches!(state.logins.get(&id), Some(Some(_)));
                if ended {
                    login = None;
                    match state.logins.remove(&id).flatten() {
                        Some(Ok(connection)) => {
                            state.busy += 1;
                            return Ok(self.leased(connection, true));
                        }
                        // The limit was learned from the refusal: decide again.
                        Some(Err(error)) if refusal_code(&error).is_some() => continue,
                        Some(Err(error)) => return Err(error),
                        None => continue,
                    }
                }
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            match capacity(state.limit) {
                Some(0) => return Ok(self.leased(self.primary.clone(), false)),
                Some(capacity) if state.busy + state.opening >= capacity => return Err(pool_full()),
                _ => {}
            }
            let id = state.next_login;
            state.next_login += 1;
            state.opening += 1;
            state.logins.insert(id, None);
            login = Some(id);
            drop(state);
            if let Err(connected) = self.start_login(id) {
                // No thread for the login: it runs here, as before.
                self.login_done(id, connected());
            }
            state = lock(&self.state);
        }
    }

    /// Starts login `id` on a thread of its own; when the thread cannot be
    /// started, hands back the login to run on the caller's thread.
    fn start_login(
        self: &Arc<Self>,
        id: u64,
    ) -> Result<(), impl FnOnce() -> io::Result<Arc<FtpConnection>>> {
        let connect = self.connect.clone();
        let run = move || {
            let attempt = std::panic::AssertUnwindSafe(|| {
                (connect)().and_then(|stream| FtpConnection::new(stream, connect.clone()))
            });
            std::panic::catch_unwind(attempt)
                .unwrap_or_else(|_| Err(io::Error::other("FTP-Anmeldung brach ab")))
        };
        let pool = Arc::downgrade(self);
        let fallback = run.clone();
        let started = std::thread::Builder::new()
            .name("ftp-login".to_string())
            .spawn(move || {
                let connected = run();
                if let Some(pool) = pool.upgrade() {
                    pool.login_done(id, connected);
                }
            });
        started.map(|_| ()).map_err(|_| fallback)
    }

    /// Starts logins in the background until `count` connections are free
    /// or on their way, within the server's known limit; each joins the free
    /// connections when it is ready (`login_done`). Never waits: the probes
    /// of a listing take them as they come (listing_detail.rs).
    fn open_spares(self: &Arc<Self>, count: usize) {
        loop {
            let mut state = lock(&self.state);
            let on_the_way = state.idle.len() + state.opening;
            let room = capacity(state.limit)
                .is_none_or(|capacity| state.busy + state.idle.len() + state.opening < capacity);
            if on_the_way >= count || !room {
                return;
            }
            let id = state.next_login;
            state.next_login += 1;
            state.opening += 1;
            drop(state);
            if self.start_login(id).is_err() {
                // No thread for it: no spare, nothing waits for one.
                let mut state = lock(&self.state);
                state.opening = state.opening.saturating_sub(1);
                return;
            }
        }
    }

    /// Login `id` ended: its lease takes the result, or, when that lease
    /// took another connection meanwhile, a new connection joins the free
    /// ones. A refusal teaches the server's limit either way.
    fn login_done(&self, id: u64, connected: io::Result<Arc<FtpConnection>>) {
        let mut state = lock(&self.state);
        state.opening = state.opening.saturating_sub(1);
        match &connected {
            Ok(connection) => connection.inherit_server_facts(&self.primary),
            Err(error) if refusal_code(error).is_some() => {
                let accepted = 1 + state.busy + state.idle.len();
                state.limit = Some(state.limit.map_or(accepted, |known| known.min(accepted)));
            }
            Err(_) => {}
        }
        match state.logins.get_mut(&id) {
            Some(slot) => *slot = Some(connected),
            None => {
                if let Ok(connection) = connected {
                    let room = capacity(state.limit)
                        .is_none_or(|capacity| state.busy + state.idle.len() < capacity);
                    if room {
                        state.idle.push((connection, Instant::now()));
                    }
                }
            }
        }
        drop(state);
        self.changed.notify_all();
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
        drop(state);
        self.changed.notify_all();
    }
}

/// A pooled connection that had to log in again and was turned away: the
/// server's connection limit, so the transfer backs off (plan K13). A
/// control or data connection that found no free local address within its
/// connect time (`local_address::connect_until`) backs off the same way: the
/// file waits and runs again with fewer transfers at once, instead of
/// failing for a port that frees up shortly. A STOR whose data connection
/// never came may have left its private stage (`stor_unconfirmed` says so);
/// the next attempt writes a new stage, the destination is untouched.
pub(super) fn transfer_err(error: io::Error) -> io::Error {
    if refusal_code(&error).is_some() {
        return crate::vfs::congestion_error(error.to_string(), None);
    }
    if super::local_address::local_address_shortage(&error) {
        return crate::vfs::congestion_error(
            format!("Keine freie lokale Adresse für die FTP-Verbindung, die Datei wartet: {error}"),
            None,
        );
    }
    error
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

/// The pool's free connections help with the probes of one listing.
impl super::listing_detail::ProbeHelpers for Arc<FtpPool> {
    fn prepare(&self, count: usize) {
        self.open_spares(count);
    }

    fn take(&self) -> Option<FtpLease> {
        self.lease_idle()
    }
}
