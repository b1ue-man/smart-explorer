//! Cancellation and progress of the terminal tree copy (`se cp`, `se mv`) for
//! callers that run it in the background, such as the MCP server's transfer
//! jobs. The terminal commands use an observer that is never cancelled, so
//! their behavior and output stay unchanged. A copy that runs on the transfer
//! engine (`engine_copy`) reports its counters and its limiting resource here
//! too, so the MCP progress and `--stats` read one place.
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

pub(super) const TRANSFER_CANCELLED: &str = "transfer cancelled";

/// Stage of one tree transfer, in execution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TreePhase {
    Planning = 0,
    Reading = 1,
    Writing = 2,
    RemovingSource = 3,
    Done = 4,
}

impl TreePhase {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Reading,
            2 => Self::Writing,
            3 => Self::RemovingSource,
            4 => Self::Done,
            _ => Self::Planning,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Reading => "reading",
            Self::Writing => "writing",
            Self::RemovingSource => "removing_source",
            Self::Done => "done",
        }
    }
}

/// Counters a background caller reads while the transfer runs. Reading copies
/// every source file into a private local spool first; writing publishes the
/// spooled files at the destination.
#[derive(Default)]
pub(super) struct TreeObserver {
    cancel: AtomicBool,
    phase: AtomicU8,
    files_total: AtomicU64,
    bytes_total: AtomicU64,
    directories_total: AtomicU64,
    files_read: AtomicU64,
    bytes_read: AtomicU64,
    files_written: AtomicU64,
    bytes_written: AtomicU64,
    /// Set once the copy runs on the transfer engine.
    engine: AtomicBool,
    engine_elapsed_ms: AtomicU64,
    engine_rate_bps: AtomicU64,
    engine_segments: AtomicU32,
    engine_verdict: Mutex<EngineVerdict>,
}

/// The text part of the engine's verdict: the stable code of the limiting
/// resource and the engine's own description of it.
#[derive(Default)]
struct EngineVerdict {
    limiter: String,
    limiter_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TreeProgress {
    pub(super) phase: TreePhase,
    pub(super) files_total: u64,
    pub(super) bytes_total: u64,
    pub(super) directories_total: u64,
    pub(super) files_read: u64,
    pub(super) bytes_read: u64,
    pub(super) files_written: u64,
    pub(super) bytes_written: u64,
    /// The copy ran on the transfer engine.
    pub(super) engine: bool,
    pub(super) elapsed_ms: u64,
    /// Throughput over the last seconds, bytes per second (engine only).
    pub(super) rate_bps: u64,
    /// `Limiter::code()` of the engine's verdict; `none` without one.
    pub(super) limiter: String,
    /// German description of the verdict (drives and rates, never a file
    /// name); `None` without a verdict.
    pub(super) limiter_text: Option<String>,
    /// Ranges of large files transferred right now (engine only).
    pub(super) segments: u32,
}

#[derive(Clone, Copy)]
pub(super) enum Counter {
    Read,
    Written,
}

impl TreeObserver {
    pub(super) fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Fails with [`TRANSFER_CANCELLED`] once cancellation was requested.
    pub(super) fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err(TRANSFER_CANCELLED.to_string())
        } else {
            Ok(())
        }
    }

    /// Replaces an error caused by cancellation with the stable message.
    pub(super) fn explain(&self, error: String) -> String {
        if self.is_cancelled() {
            TRANSFER_CANCELLED.to_string()
        } else {
            error
        }
    }

    pub(super) fn set_phase(&self, phase: TreePhase) {
        self.phase.store(phase as u8, Ordering::SeqCst);
    }

    pub(super) fn planned(&self, files: u64, directories: u64, bytes: u64) {
        self.files_total.store(files, Ordering::SeqCst);
        self.directories_total.store(directories, Ordering::SeqCst);
        self.bytes_total.store(bytes, Ordering::SeqCst);
    }

    pub(super) fn file_done(&self, counter: Counter) {
        match counter {
            Counter::Read => self.files_read.fetch_add(1, Ordering::SeqCst),
            Counter::Written => self.files_written.fetch_add(1, Ordering::SeqCst),
        };
    }

    fn bytes_done(&self, counter: Counter, bytes: u64) {
        match counter {
            Counter::Read => self.bytes_read.fetch_add(bytes, Ordering::SeqCst),
            Counter::Written => self.bytes_written.fetch_add(bytes, Ordering::SeqCst),
        };
    }

    /// The flag the transfer engine watches; setting it is `cancel`.
    pub(super) fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// The copy runs on the transfer engine from now on.
    pub(super) fn begin_engine(&self) {
        self.engine.store(true, Ordering::SeqCst);
    }

    /// What the engine has found so far (it keeps growing while it walks).
    pub(super) fn set_engine_totals(&self, files: u64, dirs: u64, bytes: u64) {
        self.planned(files, dirs, bytes);
    }

    /// The engine reads and writes in one operation, so both pairs of
    /// counters show the same numbers.
    pub(super) fn set_engine_done(&self, files: u64, bytes: u64) {
        self.files_read.store(files, Ordering::SeqCst);
        self.files_written.store(files, Ordering::SeqCst);
        self.bytes_read.store(bytes, Ordering::SeqCst);
        self.bytes_written.store(bytes, Ordering::SeqCst);
    }

    pub(super) fn set_engine_elapsed(&self, elapsed_ms: u64) {
        self.engine_elapsed_ms.store(elapsed_ms, Ordering::SeqCst);
    }

    /// Rate and verdict of the engine's latest progress. `limiter` is a
    /// `Limiter::code()`.
    pub(super) fn set_engine_stats(
        &self,
        rate_bps: u64,
        limiter: &str,
        limiter_text: Option<String>,
        segments: u32,
    ) {
        self.engine_rate_bps.store(rate_bps, Ordering::SeqCst);
        self.engine_segments.store(segments, Ordering::SeqCst);
        let mut verdict = self
            .engine_verdict
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        verdict.limiter = limiter.to_string();
        verdict.limiter_text = limiter_text;
    }

    pub(super) fn progress(&self) -> TreeProgress {
        let (limiter, limiter_text) = {
            let verdict = self
                .engine_verdict
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let code = if verdict.limiter.is_empty() {
                "none".to_string()
            } else {
                verdict.limiter.clone()
            };
            (code, verdict.limiter_text.clone())
        };
        TreeProgress {
            phase: TreePhase::from_u8(self.phase.load(Ordering::SeqCst)),
            files_total: self.files_total.load(Ordering::SeqCst),
            bytes_total: self.bytes_total.load(Ordering::SeqCst),
            directories_total: self.directories_total.load(Ordering::SeqCst),
            files_read: self.files_read.load(Ordering::SeqCst),
            bytes_read: self.bytes_read.load(Ordering::SeqCst),
            files_written: self.files_written.load(Ordering::SeqCst),
            bytes_written: self.bytes_written.load(Ordering::SeqCst),
            engine: self.engine.load(Ordering::SeqCst),
            elapsed_ms: self.engine_elapsed_ms.load(Ordering::SeqCst),
            rate_bps: self.engine_rate_bps.load(Ordering::SeqCst),
            limiter,
            limiter_text,
            segments: self.engine_segments.load(Ordering::SeqCst),
        }
    }

    pub(super) fn reader<R: Read>(&self, inner: R, counter: Counter) -> ObservedRead<'_, R> {
        ObservedRead {
            inner,
            observer: self,
            counter,
        }
    }
}

/// Counts the bytes of one file copy and stops it once the transfer was
/// cancelled. The error kind is `Other`, never `Interrupted`, because
/// `io::copy` retries interrupted reads.
pub(super) struct ObservedRead<'a, R> {
    inner: R,
    observer: &'a TreeObserver,
    counter: Counter,
}

impl<R: Read> Read for ObservedRead<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.observer.is_cancelled() {
            return Err(io::Error::other(TRANSFER_CANCELLED));
        }
        let read = self.inner.read(buffer)?;
        self.observer.bytes_done(self.counter, read as u64);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::{Counter, TreeObserver, TreePhase, TRANSFER_CANCELLED};
    use std::io::Read;
    use std::sync::atomic::Ordering;

    #[test]
    fn mcp_task_observed_reads_count_bytes_and_stop_after_cancel() {
        let observer = TreeObserver::default();
        observer.set_phase(TreePhase::Reading);
        let mut reader = observer.reader(&b"abcdef"[..], Counter::Read);
        let mut first = [0u8; 4];
        assert_eq!(reader.read(&mut first).unwrap(), 4);
        observer.cancel();
        let error = reader.read(&mut first).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), TRANSFER_CANCELLED);
        let progress = observer.progress();
        assert_eq!(progress.phase, TreePhase::Reading);
        assert_eq!(progress.bytes_read, 4);
        assert_eq!(observer.explain("other".into()), TRANSFER_CANCELLED);
    }

    #[test]
    fn speed_task_cli_observer_maps_engine_counters() {
        let observer = TreeObserver::default();
        let idle = observer.progress();
        assert!(!idle.engine);
        assert_eq!(idle.limiter, "none");
        assert_eq!(idle.limiter_text, None);
        assert_eq!((idle.rate_bps, idle.segments, idle.elapsed_ms), (0, 0, 0));

        observer.begin_engine();
        observer.set_engine_totals(7, 2, 9_000);
        observer.set_engine_done(3, 4_096);
        observer.set_engine_elapsed(1_500);
        observer.set_engine_stats(
            2_048,
            "remote",
            Some("Gegenstelle (SFTP-Server)".to_string()),
            4,
        );
        let progress = observer.progress();
        assert!(progress.engine);
        assert_eq!(progress.files_total, 7);
        assert_eq!(progress.directories_total, 2);
        assert_eq!(progress.bytes_total, 9_000);
        // Reading and writing are one operation on the engine.
        assert_eq!((progress.files_read, progress.files_written), (3, 3));
        assert_eq!(
            (progress.bytes_read, progress.bytes_written),
            (4_096, 4_096)
        );
        assert_eq!(progress.elapsed_ms, 1_500);
        assert_eq!(progress.rate_bps, 2_048);
        assert_eq!(progress.limiter, "remote");
        assert_eq!(
            progress.limiter_text.as_deref(),
            Some("Gegenstelle (SFTP-Server)")
        );
        assert_eq!(progress.segments, 4);

        // The engine watches the flag that `cancel` sets.
        assert!(!observer.cancel_flag().load(Ordering::SeqCst));
        observer.cancel();
        assert!(observer.cancel_flag().load(Ordering::SeqCst));
    }
}
