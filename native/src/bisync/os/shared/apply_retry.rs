use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const RETRY_POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailurePhase {
    PreCommit,
    CommitAttempted,
}

#[derive(Debug)]
pub(super) struct AttemptError {
    error: io::Error,
    phase: FailurePhase,
}

impl AttemptError {
    pub(super) fn pre_commit(error: io::Error) -> Self {
        Self {
            error,
            phase: FailurePhase::PreCommit,
        }
    }

    pub(super) fn commit_attempted(error: io::Error) -> Self {
        Self {
            error,
            phase: FailurePhase::CommitAttempted,
        }
    }

    fn retryable(&self, cancel: &AtomicBool) -> bool {
        self.phase == FailurePhase::PreCommit
            && !cancel.load(Ordering::Relaxed)
            && is_transient(self.error.kind())
    }

    pub(super) fn into_io(self) -> io::Error {
        self.error
    }
}

pub(super) fn run_with_retry<T>(
    retries: u32,
    delay: Duration,
    cancel: &AtomicBool,
    mut operation: impl FnMut() -> Result<T, AttemptError>,
) -> Result<T, AttemptError> {
    let mut retries_used = 0u32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AttemptError::pre_commit(interrupted()));
        }
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if retries_used < retries && error.retryable(cancel) => {
                retries_used += 1;
                wait_or_cancel(delay, cancel).map_err(AttemptError::pre_commit)?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_transient(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
            | io::ErrorKind::Interrupted
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::NetworkDown
            | io::ErrorKind::StaleNetworkFileHandle
    )
}

fn wait_or_cancel(delay: Duration, cancel: &AtomicBool) -> io::Result<()> {
    let Some(deadline) = Instant::now().checked_add(delay) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "retry delay is out of range",
        ));
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(interrupted());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        std::thread::sleep(remaining.min(RETRY_POLL));
    }
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "synchronization retry canceled")
}

#[cfg(test)]
#[path = "apply_retry_tests.rs"]
mod tests;
