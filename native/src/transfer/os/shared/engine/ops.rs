//! One file operation between the two sides of a job, under the permits the
//! worker took: the shared result and error types, the byte meter, the state a
//! retry continues from, and the dispatch per endpoint combination.
use super::super::flow::PermitPair;
use super::super::limiter::OpAccounts;
use super::queue::FileWork;
use super::stats::{RunningSegment, Stats};
use super::view::Side;
use super::Engine;
use std::cell::Cell;
use std::io;
use std::time::{Duration, Instant};

/// Copy buffer per running operation: the largest chunk any backend moves
/// per call (agent `CHUNK` 256 KiB, SFTP READ/WRITE ≈ 255 KiB); a bigger
/// buffer gains nothing because every protocol splits into its own chunks.
pub(crate) const COPY_BUFFER: usize = 256 * 1024;
/// Chunks a remote-to-remote stream holds between its reader and writer
/// (1 MiB): it absorbs the latency jitter of two independent connections
/// while each side's protocol window does the real pipelining.
pub(crate) const STREAM_DEPTH: usize = 4;

/// How a file operation ended when it did not fail.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Done,
    /// Left alone by the conflict policy or already present (resume).
    Skipped,
}

/// Where an operation failed, which decides what may follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum At {
    /// Reading the source; nothing was published.
    Source,
    /// Creating or writing the private stage or part file; nothing was
    /// published.
    Target,
    /// Publishing, known to have published nothing (names used up, a local
    /// rename refused).
    Publish,
    /// The result is unknown: the file may exist at the target. Never
    /// retried, so no duplicate "Name (2)" appears.
    Unknown,
}

#[derive(Debug)]
pub(crate) struct OpError {
    pub error: io::Error,
    pub at: At,
    /// A one-request publication under the planned name found the name
    /// taken and published nothing (G3-08b): the file is tried again at
    /// once on the staged path, without using up its retry.
    pub absent_taken: bool,
}

impl OpError {
    pub(crate) fn at(at: At, error: io::Error) -> Self {
        Self {
            error,
            at,
            absent_taken: false,
        }
    }

    /// The name a one-request publication wanted is taken; nothing of this
    /// attempt is left under it (`publish::complete`, G3-08b).
    pub(crate) fn absent_taken(error: io::Error) -> Self {
        Self {
            absent_taken: true,
            ..Self::at(At::Target, error)
        }
    }

    pub(crate) fn source(error: io::Error) -> Self {
        Self::at(At::Source, error)
    }

    pub(crate) fn target(error: io::Error) -> Self {
        Self::at(At::Target, error)
    }

    pub(crate) fn canceled() -> Self {
        Self::at(
            At::Target,
            io::Error::new(
                io::ErrorKind::Interrupted,
                super::super::cancel::CANCELED_ERROR,
            ),
        )
    }
}

pub(crate) type OpResult = Result<Outcome, OpError>;

/// `error` under `prefix` for one more file; a congestion stays one, so the
/// file still waits for the peer instead of failing (K13).
pub(crate) fn relabeled(error: &io::Error, prefix: &str) -> io::Error {
    match crate::vfs::congestion_of(error) {
        Some(congestion) => crate::vfs::congestion_error(
            format!("{prefix}: {}", congestion.message),
            congestion.retry_after,
        ),
        None => io::Error::new(error.kind(), format!("{prefix}: {error}")),
    }
}

/// Counts streamed bytes for the flow controller and the progress, and the
/// time the operation spends by account (source, target, computing, waiting):
/// the verdict "who limits" (`core/limiter.rs`) is made from these. Every
/// charge also goes to the job's totals at once, so an operation that runs
/// for a minute already shows in the verdict of the next second.
pub(crate) struct Meter<'m> {
    permits: &'m PermitPair,
    stats: &'m Stats,
    moved: Cell<u64>,
    accounts: Cell<OpAccounts>,
}

/// The error of a thread the process refused to start: back-pressure like a
/// busy peer (`WouldBlock` counts as overload), so the file waits and the
/// flow halves instead of the file failing.
pub(crate) fn thread_refusal(error: io::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        format!("Der Prozess kann keinen weiteren Thread starten: {error}"),
    )
}

/// Nanoseconds of a duration, saturating.
pub(crate) fn nanos(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
}

/// The account a measured stretch of time belongs to.
#[derive(Clone, Copy)]
enum Account {
    Source,
    Target,
    Cpu,
}

impl<'m> Meter<'m> {
    pub(crate) fn new(permits: &'m PermitPair, stats: &'m Stats) -> Self {
        Self {
            permits,
            stats,
            moved: Cell::new(0),
            accounts: Cell::new(OpAccounts::default()),
        }
    }

    fn timed<T>(&self, account: Account, f: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = f();
        self.charge(account, nanos(started.elapsed()));
        result
    }

    fn charge(&self, account: Account, ns: u64) {
        let delta = match account {
            Account::Source => OpAccounts {
                source_ns: ns,
                ..OpAccounts::default()
            },
            Account::Target => OpAccounts {
                target_ns: ns,
                ..OpAccounts::default()
            },
            Account::Cpu => OpAccounts {
                cpu_ns: ns,
                ..OpAccounts::default()
            },
        };
        self.accounts.set(self.accounts.get().plus(delta));
        self.stats.book(delta);
    }

    /// Runs a read call of the source and books its time to the source.
    pub(crate) fn timed_source<T>(&self, f: impl FnOnce() -> T) -> T {
        self.timed(Account::Source, f)
    }

    /// Runs a write call of the target and books its time to the target.
    pub(crate) fn timed_target<T>(&self, f: impl FnOnce() -> T) -> T {
        self.timed(Account::Target, f)
    }

    /// Runs a hash, compression or encryption step of this thread.
    pub(crate) fn timed_cpu<T>(&self, f: impl FnOnce() -> T) -> T {
        self.timed(Account::Cpu, f)
    }

    /// Source time spent on another thread of this operation (the reader of
    /// a remote-to-remote stream).
    pub(crate) fn add_source_ns(&self, ns: u64) {
        self.charge(Account::Source, ns);
    }

    /// Target time that was not measured call by call (the wire time of a
    /// packet, which no single write call shows).
    pub(crate) fn add_target_ns(&self, ns: u64) {
        self.charge(Account::Target, ns);
    }

    /// Time the thread waited for permit, memory or a folder before it began
    /// (or between two attempts).
    pub(crate) fn add_wait(&self, ns: u64) {
        let delta = OpAccounts {
            wait_ns: ns,
            ..OpAccounts::default()
        };
        self.accounts.set(self.accounts.get().plus(delta));
        self.stats.book(delta);
    }

    /// What this operation spent so far, by account.
    pub(crate) fn accounts(&self) -> OpAccounts {
        self.accounts.get()
    }

    /// Counts a range of a large file as running while the guard lives.
    pub(crate) fn segment(&self) -> RunningSegment<'m> {
        self.stats.begin_segment()
    }

    pub(crate) fn add(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.permits.progress(bytes);
        self.stats.moved(bytes);
        self.moved.set(self.moved.get().saturating_add(bytes));
    }

    /// Bytes an earlier attempt already delivered (a resumed download): file
    /// progress again, but no new work for the flow.
    pub(crate) fn credit(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.stats.credit(bytes);
        self.moved.set(self.moved.get().saturating_add(bytes));
    }

    /// Takes back a `credit` the attempt will not keep (a download that
    /// began in parts and went on as one stream credits the same bytes again).
    pub(crate) fn uncredit(&self, bytes: u64) {
        self.stats.unmoved(bytes);
        self.moved.set(self.moved.get().saturating_sub(bytes));
    }

    /// Bytes helper workers of a split download already reported (progress
    /// and flow saw them there): counted here only so that a failed attempt
    /// gives back everything the file counted.
    pub(crate) fn absorb(&self, bytes: u64) {
        self.moved.set(self.moved.get().saturating_add(bytes));
    }

    /// Work the flow sees that is not file progress (the first leg of a
    /// bridged copy).
    pub(crate) fn add_hidden(&self, bytes: u64) {
        self.permits.progress(bytes);
    }

    pub(crate) fn moved(&self) -> u64 {
        self.moved.get()
    }
}

/// What a retry of the same file continues from (K8): the local part file of
/// a download and how much of it is written. Dropping it removes the part.
#[derive(Default)]
pub(crate) struct Carry {
    pub part: Option<super::download::Part>,
}

/// Runs one file operation for `file` on the job's endpoints.
pub(crate) fn transfer<'e>(
    engine: &Engine<'e>,
    file: &FileWork,
    parent_created: bool,
    carry: &mut Carry,
    meter: &Meter<'_>,
    buffer: &mut [u8],
) -> OpResult {
    if let Some(outcome) = super::file_recovery::resume(engine, file)? {
        return Ok(outcome);
    }
    crate::source_media::with_cancellation(engine.stop_flag(), || {
        match (engine.view.source, engine.view.target) {
            (Side::Local, Side::Local) => {
                super::local::copy_file(engine, file, parent_created, meter)
            }
            (Side::Local, Side::Remote(target)) => {
                super::upload::upload(engine, target, file, parent_created, meter, buffer)
            }
            (Side::Remote(source), Side::Local) => {
                super::download::download(engine, source, file, carry, meter, buffer)
            }
            (Side::Remote(source), Side::Remote(target)) => super::remote_copy::copy(
                engine,
                super::remote_copy::Pair { source, target },
                file,
                parent_created,
                meter,
            ),
        }
    })
}

/// Memory an operation of this job buffers while it runs (K7).
pub(crate) fn reservation(engine: &Engine<'_>) -> u64 {
    match (engine.view.source, engine.view.target) {
        (Side::Remote(_), Side::Remote(_)) => ((STREAM_DEPTH + 1) * COPY_BUFFER) as u64,
        _ => COPY_BUFFER as u64,
    }
}
