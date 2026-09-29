//! Overload is not a failure. With many operations in flight, pooled
//! connections answer "busy" (FTP and SFTP pools, WebDAV 429/503, a Share
//! peer's `Busy`, Drive rate limits) through `vfs::congestion_error`, and a
//! stalled connection times out. Like the transfer engine, a sync gives the
//! permits back as overload (the flow halves its limit), waits the peer's
//! delay without holding any permit, takes permits again and repeats the
//! operation, as long as nothing was published yet and the run keeps making
//! progress. Only then is the error reported.
use crate::transfer::engine::{jitter, sleep_unless};
use crate::transfer::engine_policy::{retry_delay, RETRY_AFTER_MAX};
use crate::transfer::{classify_error, FlowPermit, OpOutcome, PermitPair};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Overload may go on this long without any operation of the run getting
/// through, and one operation may be refused this long, before the error is
/// reported: five times the longest "retry after" a peer is honored for
/// (`RETRY_AFTER_MAX`, one minute), so a rate limit that lifts is always
/// waited out, while a peer that grants nothing for five minutes counts as
/// unavailable (the next run repeats what failed).
pub(crate) const OVERLOAD_PATIENCE: Duration = Duration::from_secs(RETRY_AFTER_MAX.as_secs() * 5);

/// When the run last got an operation through; shared by all its workers.
pub(crate) struct Progress {
    epoch: Instant,
    last_ms: AtomicU64,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }
}

impl Progress {
    /// An operation of the run succeeded.
    pub(crate) fn touch(&self) {
        let now = self.epoch.elapsed().as_millis() as u64;
        self.last_ms.fetch_max(now, Ordering::Relaxed);
    }

    fn idle(&self) -> Duration {
        let now = self.epoch.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.last_ms.load(Ordering::Relaxed)))
    }
}

/// Backing off from overload for one operation.
pub(crate) struct Backoff<'r> {
    progress: &'r Progress,
    since: Option<Instant>,
    patience: Duration,
}

impl<'r> Backoff<'r> {
    pub(crate) fn new(progress: &'r Progress) -> Self {
        Self {
            progress,
            since: None,
            patience: OVERLOAD_PATIENCE,
        }
    }

    /// A shorter patience (tests).
    pub(crate) fn with_patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// The pause before trying again after `error`: the peer's own delay or
    /// the engine's jittered base delay. `None` when the error is no overload
    /// or patience ran out (for this operation or for the whole run).
    pub(crate) fn pause(&mut self, error: &io::Error) -> Option<Duration> {
        if classify_error(error) != OpOutcome::Overload {
            return None;
        }
        let since = *self.since.get_or_insert_with(Instant::now);
        if since.elapsed() >= self.patience || self.progress.idle() >= self.patience {
            return None;
        }
        let retry_after =
            crate::vfs::congestion_of(error).and_then(|congestion| congestion.retry_after);
        Some(retry_delay(retry_after, jitter()))
    }
}

/// The permits of one operation, ended with its outcome.
pub(crate) trait Permits {
    fn end(self, outcome: OpOutcome);
}

impl Permits for FlowPermit {
    fn end(self, outcome: OpOutcome) {
        self.finish(outcome);
    }
}

impl Permits for PermitPair {
    fn end(self, outcome: OpOutcome) {
        self.finish(outcome);
    }
}

/// A listing permit of a side that does not regulate is `None`.
impl Permits for Option<FlowPermit> {
    fn end(self, outcome: OpOutcome) {
        if let Some(permit) = self {
            permit.finish(outcome);
        }
    }
}

/// Runs a read-only operation (listing, `stat`, checksum read) under permits
/// from `acquire`, again after overload as described above. `None` once
/// canceled, before or between attempts.
pub(crate) fn under_permits<P: Permits, T>(
    cancel: &AtomicBool,
    progress: &Progress,
    mut acquire: impl FnMut() -> Option<P>,
    mut attempt: impl FnMut(&P) -> io::Result<T>,
) -> Option<io::Result<T>> {
    let mut backoff = Backoff::new(progress);
    loop {
        let permits = acquire()?;
        let result = attempt(&permits);
        permits.end(match &result {
            Ok(_) => OpOutcome::Done,
            Err(error) => classify_error(error),
        });
        let delay = match &result {
            Ok(_) => None,
            Err(error) => backoff.pause(error),
        };
        if result.is_ok() {
            progress.touch();
        }
        let Some(delay) = delay else {
            return Some(result);
        };
        if !sleep_unless(cancel, delay) {
            return None;
        }
    }
}

#[cfg(test)]
#[path = "sync_overload_tests.rs"]
mod tests;
