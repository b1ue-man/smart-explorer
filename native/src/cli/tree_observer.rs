//! Cancellation and progress of the terminal tree copy (`se cp`, `se mv`) for
//! callers that run it in the background, such as the MCP server's transfer
//! jobs. The terminal commands use an observer that is never cancelled, so
//! their behavior and output stay unchanged.
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TreeProgress {
    pub(super) phase: TreePhase,
    pub(super) files_total: u64,
    pub(super) bytes_total: u64,
    pub(super) directories_total: u64,
    pub(super) files_read: u64,
    pub(super) bytes_read: u64,
    pub(super) files_written: u64,
    pub(super) bytes_written: u64,
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

    pub(super) fn progress(&self) -> TreeProgress {
        TreeProgress {
            phase: TreePhase::from_u8(self.phase.load(Ordering::SeqCst)),
            files_total: self.files_total.load(Ordering::SeqCst),
            bytes_total: self.bytes_total.load(Ordering::SeqCst),
            directories_total: self.directories_total.load(Ordering::SeqCst),
            files_read: self.files_read.load(Ordering::SeqCst),
            bytes_read: self.bytes_read.load(Ordering::SeqCst),
            files_written: self.files_written.load(Ordering::SeqCst),
            bytes_written: self.bytes_written.load(Ordering::SeqCst),
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
}
