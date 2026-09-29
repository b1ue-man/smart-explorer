//! Extra SFTP channels for file transfers on the backend's SSH connection.
//! One channel is one `sftp-server` process with its own SSH window (OpenSSH
//! grants 2 MiB per session channel), so files spread over several channels
//! move in parallel on the server and uploads get one window each. Listing
//! and metadata stay on the main session.
//!
//! A transfer leases the least busy channel; while every channel is busy a
//! new one is opened, as rclone opens one SFTP session per concurrent
//! transfer. Idle channels close after a minute, except the last one used.
//! The server's limit is learned from its first refusal (OpenSSH:
//! `MaxSessions`, default 10, answered with "open failed"): the pool then
//! keeps two channels free for the agent's exec channel and the short-lived
//! posix-rename channel (plan K25) and shares its channels instead, since
//! SFTP multiplexes requests of many files on one channel. When no channel
//! can be opened at all, transfers use the main session as before.
use super::backend::SftpBackend;
use super::connection::{classify_sftp_error, SftpConnection, SftpGeneration};
use super::exec::ChannelOpen;
use super::io_err;
use super::posix_rename::POSIX_RENAME;
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::rawsession::Limits;
use russh_sftp::client::RawSftpSession;
use russh_sftp::extensions;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

/// Channels left free after a refusal: the agent's exec channel and the
/// short-lived posix-rename channel (plan K25).
const RESERVED_CHANNELS: usize = 2;
/// A channel unused this long belongs to a finished burst; rclone empties
/// its SFTP pool after the same idle time (`--sftp-idle-timeout` 1m0s).
const IDLE_RETIRE: Duration = Duration::from_secs(60);
/// Subsystem request, INIT and limits of a new channel, the same bound as
/// the posix-rename channel's subsystem setup.
const SETUP_DEADLINE: Duration = Duration::from_secs(10);
/// Answer deadline per request. The pipeline keeps its queue within about one
/// unloaded round trip, so an answer this late means a stalled channel; the
/// same inactivity bound as FTP and WebDAV transfers.
const REQUEST_TIMEOUT_SECS: u64 = 60;
/// russh-sftp's `Config::max_packet_len` default, the cap of every request.
const CLIENT_MAX_PACKET: u64 = 262_144;
/// SSH_FXP_DATA header: type, id and length (russh-sftp file.rs).
const READ_OVERHEAD: u64 = 9;
/// SSH_FXP_WRITE without the handle: type, id, handle length, offset and
/// data length (russh-sftp file.rs).
const WRITE_OVERHEAD: u64 = 21;

pub(super) struct PoolChannel {
    pub(super) session: Arc<RawSftpSession>,
    pub(super) generation: Arc<SftpGeneration>,
    packet_len: u64,
    read_len: u64,
    write_len: Option<u64>,
    /// `fsync@openssh.com` offered: flushes sync like `File::flush`.
    pub(super) fsync: bool,
    /// `posix-rename@openssh.com` offered: an atomic replace needs no extra
    /// channel then (posix_rename.rs).
    pub(super) posix_rename: bool,
    active: AtomicUsize,
    idle_since: Mutex<Instant>,
    broken: AtomicBool,
}

impl PoolChannel {
    fn new(setup: Setup, generation: Arc<SftpGeneration>) -> Self {
        let packet_len = setup
            .limits
            .and_then(|limits| limits.packet_len)
            .map_or(CLIENT_MAX_PACKET, |packet| packet.min(CLIENT_MAX_PACKET));
        let fit = packet_len.saturating_sub(READ_OVERHEAD).max(1);
        let read_len = setup
            .limits
            .and_then(|limits| limits.read_len)
            .map_or(fit, |read| read.min(fit));
        Self {
            session: Arc::new(setup.session),
            generation,
            packet_len,
            read_len,
            write_len: setup.limits.and_then(|limits| limits.write_len),
            fsync: setup.fsync,
            posix_rename: setup.posix_rename,
            active: AtomicUsize::new(0),
            idle_since: Mutex::new(Instant::now()),
            broken: AtomicBool::new(false),
        }
    }

    /// Bytes per READ: the server's `limits@openssh.com` read length, else
    /// what one packet carries (as `File::poll_read`).
    pub(super) fn read_chunk(&self) -> u32 {
        u32::try_from(self.read_len).unwrap_or(u32::MAX)
    }

    /// Bytes per WRITE for a handle of `handle_len` bytes.
    pub(super) fn write_chunk(&self, handle_len: usize) -> usize {
        let overhead = WRITE_OVERHEAD.saturating_add(handle_len as u64);
        let fit = self.packet_len.saturating_sub(overhead).max(1);
        let chunk = self.write_len.map_or(fit, |limit| limit.min(fit)).max(1);
        usize::try_from(chunk).unwrap_or(usize::MAX)
    }

    fn mark_broken(&self) {
        self.broken.store(true, Ordering::Release);
    }

    fn load(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*lock(&self.idle_since))
    }
}

/// One transfer's use of a channel; released on drop.
pub(super) struct ChannelLease {
    channel: Arc<PoolChannel>,
}

impl ChannelLease {
    fn new(channel: Arc<PoolChannel>) -> Self {
        channel.active.fetch_add(1, Ordering::AcqRel);
        Self { channel }
    }

    pub(super) fn channel(&self) -> &Arc<PoolChannel> {
        &self.channel
    }
}

impl Drop for ChannelLease {
    fn drop(&mut self) {
        if self.channel.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            *lock(&self.channel.idle_since) = Instant::now();
        }
    }
}

/// The failure of a request on `channel`: a closed SSH session retires the
/// whole connection (as on the main session), a dead or suspect SFTP stream
/// only this channel. `true` when replaying on another channel is safe.
pub(super) fn note_failure(
    connection: &SftpConnection,
    channel: &PoolChannel,
    error: &SftpError,
) -> bool {
    if channel.generation.session().is_closed() {
        channel.mark_broken();
        return connection.note_sftp_error(&channel.generation, error);
    }
    let disposition = classify_sftp_error(error);
    if disposition.retire {
        channel.mark_broken();
    }
    disposition.retry_safe
}

/// An OPEN that failed. The server's open-handle limit (`limits@openssh.com`,
/// checked by russh-sftp before sending) is congestion: fewer files at once
/// succeed, so the transfer's flow backs off instead of failing (plan K13).
pub(super) fn open_failed(error: SftpError) -> io::Error {
    match error {
        SftpError::Limited(message) => crate::vfs::congestion_error(
            format!("Der SFTP-Server hält keine weiteren Dateien offen ({message})"),
            None,
        ),
        error => io_err(error),
    }
}

/// Pool channels allowed after the server refused one while `open` were
/// open: two stay free for exec and posix-rename.
pub(super) fn learned_limit(open: usize) -> usize {
    open.saturating_sub(RESERVED_CHANNELS)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Pick {
    Use(usize),
    Open,
    MainSession,
}

/// The channel for a new transfer, from the channels' current `loads`: an
/// idle one; else a new one while the limit allows; else the least busy one.
/// `MainSession` when the pool may hold no channel at all.
pub(super) fn pick(loads: &[usize], opening: usize, limit: Option<usize>) -> Pick {
    let may_open = limit.is_none_or(|limit| loads.len() + opening < limit);
    let least = loads.iter().enumerate().min_by_key(|(_, load)| **load);
    match least {
        Some((index, &load)) if load == 0 || !may_open => Pick::Use(index),
        None if !may_open => Pick::MainSession,
        _ => Pick::Open,
    }
}

#[derive(Default)]
pub(super) struct ChannelPool {
    state: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    channels: Vec<Arc<PoolChannel>>,
    opening: usize,
    limit: Option<usize>,
    /// Setup of a channel failed on this generation: the main session serves.
    disabled: Option<Weak<SftpGeneration>>,
}

enum Choice {
    Use(ChannelLease),
    Open,
    MainSession,
}

impl PoolState {
    fn prune(&mut self, generation: &Arc<SftpGeneration>, now: Instant) {
        self.channels.retain(|channel| {
            Arc::ptr_eq(&channel.generation, generation) && !channel.broken.load(Ordering::Acquire)
        });
        self.retire_idle(now);
    }

    fn retire_idle(&mut self, now: Instant) {
        // The most recently used idle channel stays open, so a single later
        // file does not pay the channel setup (three round trips) again.
        let warm = self
            .channels
            .iter()
            .filter(|channel| channel.load() == 0)
            .min_by_key(|channel| channel.idle_for(now))
            .cloned();
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

    fn choose(&mut self) -> Choice {
        let loads: Vec<usize> = self.channels.iter().map(|channel| channel.load()).collect();
        match pick(&loads, self.opening, self.limit) {
            Pick::Use(index) => match self.channels.get(index) {
                Some(channel) => Choice::Use(ChannelLease::new(channel.clone())),
                None => Choice::MainSession,
            },
            Pick::Open => {
                self.opening += 1;
                Choice::Open
            }
            Pick::MainSession => Choice::MainSession,
        }
    }

    fn refused(&mut self) {
        let limit = learned_limit(self.channels.len());
        let limit = self.limit.map_or(limit, |known| known.min(limit));
        self.limit = Some(limit);
        // Busy channels first: the idle ones beyond the limit close now, a
        // busy one after its last lease (it gets no new ones).
        self.channels
            .sort_by_key(|channel| std::cmp::Reverse(channel.load()));
        self.channels.truncate(limit);
    }
}

enum Opened {
    Ready(PoolChannel),
    Refused,
    Unavailable(Arc<SftpGeneration>),
}

struct Setup {
    session: RawSftpSession,
    limits: Option<Limits>,
    fsync: bool,
    posix_rename: bool,
}

impl ChannelPool {
    /// Closes channels idle past `IDLE_RETIRE` (all but the warm one)
    /// without waiting for the next transfer.
    pub(super) fn retire_idle(&self) {
        lock(&self.state).retire_idle(Instant::now());
    }

    /// A channel for one file transfer; `None`: use the main session.
    pub(super) fn lease(&self, backend: &SftpBackend) -> io::Result<Option<ChannelLease>> {
        loop {
            let generation = backend.connection.current()?;
            {
                let mut state = lock(&self.state);
                state.prune(&generation, Instant::now());
                if state.disabled_for(&generation) {
                    return Ok(None);
                }
                match state.choose() {
                    Choice::Use(lease) => return Ok(Some(lease)),
                    Choice::MainSession => return Ok(None),
                    Choice::Open => {}
                }
            }
            let opened = open_channel(backend);
            let mut state = lock(&self.state);
            state.opening = state.opening.saturating_sub(1);
            match opened {
                Ok(Opened::Ready(channel)) => {
                    let channel = Arc::new(channel);
                    let lease = ChannelLease::new(channel.clone());
                    state.channels.push(channel);
                    return Ok(Some(lease));
                }
                Ok(Opened::Refused) => state.refused(),
                Ok(Opened::Unavailable(generation)) => {
                    state.disabled = Some(Arc::downgrade(&generation));
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn open_channel(backend: &SftpBackend) -> io::Result<Opened> {
    let (generation, channel) = match backend.try_open_session_channel()? {
        ChannelOpen::Opened(generation, channel) => (generation, channel),
        ChannelOpen::Refused(_) => return Ok(Opened::Refused),
    };
    match backend.rt.block_on(setup(channel)) {
        Ok(setup) => Ok(Opened::Ready(PoolChannel::new(setup, generation))),
        // A lost connection fails the transfer (the next call reconnects);
        // any other setup failure only turns the pool off for this session.
        Err(error) if generation.session().is_closed() => {
            backend.connection.mark_stale(&generation);
            Err(error)
        }
        Err(_) => Ok(Opened::Unavailable(generation)),
    }
}

async fn setup(channel: russh::Channel<client::Msg>) -> io::Result<Setup> {
    let staged = tokio::time::timeout(SETUP_DEADLINE, async move {
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(io_err)?;
        let mut session = RawSftpSession::new(channel.into_stream());
        session.set_timeout(REQUEST_TIMEOUT_SECS);
        let version = session.init().await.map_err(io_err)?;
        let offered = |name: &str| {
            version
                .extensions
                .get(name)
                .is_some_and(|value| value == "1")
        };
        let fsync = offered(extensions::FSYNC);
        let posix_rename = offered(POSIX_RENAME);
        let limits = if offered(extensions::LIMITS) {
            let limits = Limits::from(session.limits().await.map_err(io_err)?);
            session.set_limits(limits);
            Some(limits)
        } else {
            None
        };
        Ok::<Setup, io::Error>(Setup {
            session,
            limits,
            fsync,
            posix_rename,
        })
    })
    .await;
    staged.map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "SFTP-Übertragungskanal ließ sich nicht rechtzeitig einrichten",
        )
    })?
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
