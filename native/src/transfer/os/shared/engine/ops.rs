//! One file operation between the two sides of a job, under the permits the
//! worker took: the shared result and error types, the byte meter, the state a
//! retry continues from, and the dispatch per endpoint combination.
use super::super::flow::PermitPair;
use super::queue::FileWork;
use super::stats::Stats;
use super::view::Side;
use super::Engine;
use std::cell::Cell;
use std::io;

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
}

impl OpError {
    pub(crate) fn at(at: At, error: io::Error) -> Self {
        Self { error, at }
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

/// Counts streamed bytes for the flow controller and the progress.
pub(crate) struct Meter<'m> {
    permits: &'m PermitPair,
    stats: &'m Stats,
    moved: Cell<u64>,
}

impl<'m> Meter<'m> {
    pub(crate) fn new(permits: &'m PermitPair, stats: &'m Stats) -> Self {
        Self {
            permits,
            stats,
            moved: Cell::new(0),
        }
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
pub(crate) fn transfer(
    engine: &Engine<'_>,
    file: &FileWork,
    parent_created: bool,
    carry: &mut Carry,
    meter: &Meter<'_>,
    buffer: &mut [u8],
) -> OpResult {
    match (engine.view.source, engine.view.target) {
        (Side::Local, Side::Local) => super::local::copy_file(engine, file, meter),
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
}

/// Memory an operation of this job buffers while it runs (K7).
pub(crate) fn reservation(engine: &Engine<'_>) -> u64 {
    match (engine.view.source, engine.view.target) {
        (Side::Remote(_), Side::Remote(_)) => ((STREAM_DEPTH + 1) * COPY_BUFFER) as u64,
        _ => COPY_BUFFER as u64,
    }
}
