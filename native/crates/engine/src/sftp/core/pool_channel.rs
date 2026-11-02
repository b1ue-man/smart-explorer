//! One SFTP channel of the pool (one `sftp-server` process with its own SSH
//! window) and a transfer's lease on it. A channel belongs to the main SSH
//! connection or to an extra connection that carries nothing else
//! (extra_link.rs); a failure retires the channel and, for an extra
//! connection, only that connection.
use super::batch_ops::OPEN_HANDLES;
use super::connection::{classify_sftp_error, SftpConnection, SftpGeneration};
use super::copy_data::COPY_DATA;
use super::handle_budget::BudgetSession;
use super::io_err;
use super::pool_growth::PoolHub;
use super::posix_rename::POSIX_RENAME;
use super::session::ExtraLink;
use crate::transfer::read_pipeline::Pipeline;
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::rawsession::Limits;
use russh_sftp::client::RawSftpSession;
use russh_sftp::extensions;
use russh_sftp::protocol::Version;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

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

/// The SSH connection a channel runs on.
pub(super) enum Owner {
    Main(Arc<SftpGeneration>),
    Extra(Arc<ExtraLink>),
}

/// Handles one channel may hold at once: `min(open-handles of the server,
/// 64)`, 64 when the server names none (security answer E13-S13).
pub(super) fn handle_cap_of(open_handles: Option<u64>) -> usize {
    open_handles
        .filter(|handles| *handles >= 1)
        .map_or(OPEN_HANDLES, |handles| {
            usize::try_from(handles)
                .unwrap_or(usize::MAX)
                .min(OPEN_HANDLES)
        })
}

pub(super) struct PoolChannel {
    /// OPEN and CLOSE keep the channel's handle budget (handle_budget.rs).
    pub(super) session: Arc<BudgetSession>,
    pub(super) owner: Owner,
    hub: Arc<PoolHub>,
    packet_len: u64,
    read_len: u64,
    write_len: Option<u64>,
    /// Files the server lets one channel hold open (`limits@openssh.com`);
    /// `None` when it names none or 0.
    open_handles: Option<u64>,
    /// `fsync@openssh.com` offered: flushes sync like `File::flush`.
    pub(super) fsync: bool,
    /// `posix-rename@openssh.com` offered: an atomic replace needs no extra
    /// channel then (posix_rename.rs).
    pub(super) posix_rename: bool,
    /// `copy-data` offered: the server copies between two handles of this
    /// channel (copy_data.rs).
    pub(super) copy_data: bool,
    active: AtomicUsize,
    idle_since: Mutex<Instant>,
    broken: AtomicBool,
    /// Pipeline depth and fastest answer the last transfer on this channel
    /// reached (G3-06b): the next one starts there instead of at two.
    learned: Mutex<Option<(usize, Duration)>>,
    /// File data has moved over this channel; an extra connection's first
    /// bytes start the judgement of its effect (pool_growth.rs `carried`).
    carried: AtomicBool,
}

impl PoolChannel {
    pub(super) fn new(setup: Setup, owner: Owner, hub: Arc<PoolHub>) -> Self {
        let packet_len = setup
            .limits
            .and_then(|limits| limits.packet_len)
            .map_or(CLIENT_MAX_PACKET, |packet| packet.min(CLIENT_MAX_PACKET));
        let fit = packet_len.saturating_sub(READ_OVERHEAD).max(1);
        let read_len = setup
            .limits
            .and_then(|limits| limits.read_len)
            .map_or(fit, |read| read.min(fit));
        let open_handles = setup.limits.and_then(|limits| limits.open_handles);
        Self {
            session: Arc::new(BudgetSession::new(setup.session, open_handles)),
            owner,
            hub,
            packet_len,
            read_len,
            write_len: setup.limits.and_then(|limits| limits.write_len),
            open_handles,
            fsync: setup.fsync,
            posix_rename: setup.posix_rename,
            copy_data: setup.copy_data,
            active: AtomicUsize::new(0),
            idle_since: Mutex::new(Instant::now()),
            broken: AtomicBool::new(false),
            learned: Mutex::new(None),
            carried: AtomicBool::new(false),
        }
    }

    /// Remembers what a finished transfer's pipeline reached on this
    /// channel; a transfer without one answer teaches nothing.
    pub(super) fn learn(&self, depth: usize, fastest: Option<Duration>) {
        if let Some(fastest) = fastest {
            *lock(&self.learned) = Some((depth, fastest));
        }
    }

    /// A pipeline of `chunk`-byte requests up to `cap` in flight, started at
    /// the depth the channel's last transfer reached instead of at two, but
    /// never deeper than the `needed` requests the transfer has (G3-06b: a
    /// 1 MiB file then moves in one round trip instead of three). The depth
    /// still grows only with memory reserved for it (`Pipeline`), and the
    /// learned fastest answer keeps the queue bound of the last transfer.
    pub(super) fn seeded_pipeline(&self, chunk: u64, cap: usize, needed: u64) -> Pipeline {
        let mut pipeline = Pipeline::new(chunk, cap);
        let Some((depth, fastest)) = *lock(&self.learned) else {
            return pipeline;
        };
        let target = usize::try_from(needed).unwrap_or(usize::MAX).min(depth);
        while pipeline.depth() < target {
            let before = pipeline.depth();
            // One answer as fast as the fastest grows the depth by one,
            // exactly as a transfer that measured it would have.
            pipeline.answered(fastest, true);
            if pipeline.depth() == before {
                break;
            }
        }
        pipeline
    }

    /// Bytes per READ: the server's `limits@openssh.com` read length, else
    /// what one packet carries (as `File::poll_read`).
    pub(super) fn read_chunk(&self) -> u32 {
        u32::try_from(self.read_len).unwrap_or(u32::MAX)
    }

    /// Files the server lets this channel hold open at once, if it said so.
    pub(super) fn open_handles(&self) -> Option<u64> {
        self.open_handles
    }

    /// Handles this channel may hold at once (`handle_cap_of`).
    pub(super) fn handle_cap(&self) -> usize {
        handle_cap_of(self.open_handles)
    }

    /// Bytes per WRITE for a handle of `handle_len` bytes.
    pub(super) fn write_chunk(&self, handle_len: usize) -> usize {
        let overhead = WRITE_OVERHEAD.saturating_add(handle_len as u64);
        let fit = self.packet_len.saturating_sub(overhead).max(1);
        let chunk = self.write_len.map_or(fit, |limit| limit.min(fit)).max(1);
        usize::try_from(chunk).unwrap_or(usize::MAX)
    }

    /// File data moved over this channel; feeds the throughput window that
    /// decides about extra connections.
    pub(super) fn moved(&self, bytes: u64) {
        if bytes > 0 && self.is_extra() && !self.carried.swap(true, Ordering::AcqRel) {
            self.hub.carried();
        }
        self.hub.note(bytes);
    }

    /// No new lease and no new OPEN; a transfer waiting for a handle slot
    /// on it stops waiting (congestion: the file goes again elsewhere).
    pub(super) fn mark_broken(&self) {
        self.broken.store(true, Ordering::Release);
        self.session.retire();
    }

    /// Broken, or retired by a handle the server named unusably.
    pub(super) fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire) || self.session.is_retired()
    }

    pub(super) fn is_extra(&self) -> bool {
        matches!(self.owner, Owner::Extra(_))
    }

    /// An extra connection's channel that still works.
    pub(super) fn extra_alive(&self) -> bool {
        match &self.owner {
            Owner::Extra(link) => !self.is_broken() && !link.is_dead(),
            Owner::Main(_) => false,
        }
    }

    /// Whether this channel may serve the connection `generation`: not
    /// broken, on that generation (an extra connection was checked against
    /// the host key of the generation it was opened for) and alive.
    pub(super) fn usable_on(&self, generation: &Arc<SftpGeneration>) -> bool {
        if self.is_broken() {
            return false;
        }
        match &self.owner {
            Owner::Main(own) => Arc::ptr_eq(own, generation),
            Owner::Extra(link) => !link.is_dead() && Arc::ptr_eq(&link.generation, generation),
        }
    }

    /// Why an extra connection closes, for the log line.
    pub(super) fn note_close(&self, reason: &'static str) {
        if let Owner::Extra(link) = &self.owner {
            link.set_reason(reason);
        }
    }

    pub(super) fn load(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub(super) fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*lock(&self.idle_since))
    }
}

/// One transfer's use of a channel; released on drop.
pub(super) struct ChannelLease {
    channel: Arc<PoolChannel>,
}

impl ChannelLease {
    pub(super) fn new(channel: Arc<PoolChannel>) -> Self {
        channel.active.fetch_add(1, Ordering::AcqRel);
        channel.hub.lease_started();
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
        self.channel.hub.lease_ended();
    }
}

/// The failure of a request on `channel`: a closed SSH session retires the
/// whole connection (as on the main session; of an extra connection only
/// that one), a dead or suspect SFTP stream only this channel. `true` when
/// replaying on another channel is safe.
pub(super) fn note_failure(
    connection: &SftpConnection,
    channel: &PoolChannel,
    error: &SftpError,
) -> bool {
    match &channel.owner {
        Owner::Main(generation) => {
            if generation.session().is_closed() {
                channel.mark_broken();
                return connection.note_sftp_error(generation, error);
            }
            let disposition = classify_sftp_error(error);
            if disposition.retire {
                channel.mark_broken();
            }
            disposition.retry_safe
        }
        Owner::Extra(link) => {
            if link.is_dead() {
                link.mark_dead("Verbindung abgebrochen");
                channel.mark_broken();
                return true;
            }
            let disposition = classify_sftp_error(error);
            if disposition.retire {
                channel.mark_broken();
                link.mark_dead("SFTP-Kanal gestört");
            }
            disposition.retry_safe
        }
    }
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

pub(super) struct Setup {
    session: RawSftpSession,
    limits: Option<Limits>,
    fsync: bool,
    posix_rename: bool,
    copy_data: bool,
}

/// Whether the server's VERSION reply offers extension `name` in the
/// version these clients speak ("1" for every extension used here).
pub(super) fn offered(version: &Version, name: &str) -> bool {
    version
        .extensions
        .get(name)
        .is_some_and(|value| value == "1")
}

/// Subsystem `sftp`, INIT and limits on a freshly opened session channel.
pub(super) async fn setup(channel: russh::Channel<client::Msg>) -> io::Result<Setup> {
    let staged = tokio::time::timeout(SETUP_DEADLINE, async move {
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(io_err)?;
        let mut session = RawSftpSession::new(channel.into_stream());
        session.set_timeout(REQUEST_TIMEOUT_SECS);
        let version = session.init().await.map_err(io_err)?;
        let fsync = offered(&version, extensions::FSYNC);
        let posix_rename = offered(&version, POSIX_RENAME);
        let copy_data = offered(&version, COPY_DATA);
        let limits = if offered(&version, extensions::LIMITS) {
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
            copy_data,
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
