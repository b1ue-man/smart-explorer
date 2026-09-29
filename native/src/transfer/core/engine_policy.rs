//! Pure decisions of the transfer engine: which failures deserve the one
//! retry of a file, how long a peer that is too busy is waited for, which
//! failures end a whole job, when a run of failures means the connection is
//! gone, how long to back off, how packets of small files are sized, and the
//! recent transfer rate. The engine supplies clocks and randomness, so
//! everything here is deterministic.
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::time::Duration;

/// Pause before the one retry of a file: long enough for a dropped SSH or TLS
/// session to reconnect (a handshake takes a few round trips, well below a
/// second on common links). The engine spreads it ±50 % at random so workers
/// that failed together do not hit the peer again in the same instant.
pub(crate) const RETRY_BASE: Duration = Duration::from_secs(1);
/// A peer's own "retry after" is honored up to this long; beyond a minute the
/// user is better served by an error that "transfer missing files" repeats.
pub(crate) const RETRY_AFTER_MAX: Duration = Duration::from_secs(60);
/// A peer that answers "too busy" is waited for while the transfer still
/// moves; this long without any progress, the refused file or folder is
/// reported instead. Five of the longest waits a peer may ask for, which
/// also spans the longest short throttling window of the common services
/// (SharePoint/OneDrive count a user's requests per 5 minutes, Google Drive
/// per minute; checked 2026-09-29). Hourly volume limits and daily quotas
/// are not waited out: "transfer missing files" repeats those files later.
/// Transfers, folder creation and sync share this bound.
pub(crate) const OVERLOAD_PATIENCE: Duration = Duration::from_secs(5 * RETRY_AFTER_MAX.as_secs());
/// One lost connection fails every running operation at once and each of
/// those files is retried once, so twice the number of running operations in
/// a row without any success means the retries failed as well. The floor
/// keeps a transfer with one or two workers from giving up after a few
/// unlucky files: eight files in a row, each already retried, are no chance.
const BREAKER_FLOOR: u64 = 8;
/// The rate shown is smoothed over this span (spec: recent seconds, not the
/// average since the start).
const RATE_SPAN_MS: u64 = 3_000;
/// Packets aim at this much transfer time at the measured rate: long enough
/// that one round trip per packet is small against it, short enough that a
/// failed packet costs little and progress stays fluid (recherche §3.1).
const BATCH_TARGET_MS: f64 = 250.0;
/// Smallest useful packet (recherche §3.1: at least 256 KiB).
const MIN_BATCH_BYTES: u64 = 256 * 1024;
/// A packet should carry at least this many files; a file above a packet's
/// share goes alone (its own transfer time already dwarfs a round trip).
const FILES_PER_BATCH_SHARE: u64 = 8;

/// The operation may succeed when tried again: the connection dropped or
/// stalled, or the peer asked to slow down (`overload` is the flow's
/// classification of the same error).
pub(crate) fn is_transient(kind: ErrorKind, overload: bool) -> bool {
    overload
        || matches!(
            kind,
            ErrorKind::TimedOut
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::BrokenPipe
                | ErrorKind::UnexpectedEof
                | ErrorKind::NotConnected
        )
}

/// The peer refused the request to slow the client down (congestion, "too
/// many concurrent …", would block): back-pressure, not a failure of the
/// file. A timeout is not, it may just as well be a dead link.
pub(crate) fn is_back_pressure(kind: ErrorKind, overload: bool) -> bool {
    overload && kind != ErrorKind::TimedOut
}

/// Whether an operation the peer keeps refusing may wait again after
/// `quiet` without any progress.
pub(crate) fn overload_keeps_waiting(quiet: Duration) -> bool {
    quiet < OVERLOAD_PATIENCE
}

/// The failed request certainly took no effect: the peer refused it or it
/// was never sent. Other transient failures may have been carried out with
/// only the answer lost.
pub(crate) fn surely_not_done(kind: ErrorKind, overload: bool) -> bool {
    is_back_pressure(kind, overload) || kind == ErrorKind::NotConnected
}

/// The target cannot take any more files: full, over quota, read-only or not
/// writable for us. Every further file would fail the same way.
pub(crate) fn ends_job_at_target(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::StorageFull
            | ErrorKind::QuotaExceeded
            | ErrorKind::ReadOnlyFilesystem
            | ErrorKind::PermissionDenied
    )
}

/// A failure that says something about the connection rather than about the
/// one file (only those feed the breaker; a locked or vanished file does not).
pub(crate) fn connection_failure(kind: ErrorKind, overload: bool) -> bool {
    is_transient(kind, overload)
        || matches!(
            kind,
            ErrorKind::ConnectionRefused
                | ErrorKind::HostUnreachable
                | ErrorKind::NetworkUnreachable
                | ErrorKind::NetworkDown
        )
}

/// Listings of some protocols carry coarser times than a later stat (FTP
/// `LIST`: minutes, in server local time; others round to seconds), so a
/// source counts as modified only when both times are fine-grained and
/// differ by at least a second. A same-size rewrite within that grain stays
/// undetected; nothing unchanged is ever refused.
pub(crate) fn listed_time_differs(listed_ms: i64, now_ms: i64) -> bool {
    const MINUTE_MS: i64 = 60_000;
    const SECOND_MS: i64 = 1_000;
    let coarse = |value: i64| value == 0 || value % MINUTE_MS == 0;
    !coarse(listed_ms) && !coarse(now_ms) && (listed_ms - now_ms).abs() >= SECOND_MS
}

/// The pause before a retry: the peer's own delay when it named one, else the
/// base delay scaled by `jitter` (0.0‥1.0) into 50‥150 %.
pub(crate) fn retry_delay(retry_after: Option<Duration>, jitter: f64) -> Duration {
    match retry_after {
        Some(delay) => delay.min(RETRY_AFTER_MAX),
        None => RETRY_BASE.mul_f64(0.5 + jitter.clamp(0.0, 1.0)),
    }
}

/// Counts connection failures in a row; any success resets it.
#[derive(Debug, Default)]
pub(crate) struct Breaker {
    consecutive: u64,
}

impl Breaker {
    pub(crate) fn success(&mut self) {
        self.consecutive = 0;
    }

    /// Records one failure; true once the run shows the connection is gone.
    pub(crate) fn failure(&mut self, running: usize) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        self.consecutive >= breaker_threshold(running)
    }
}

pub(crate) fn breaker_threshold(running: usize) -> u64 {
    (running as u64).saturating_mul(2).max(BREAKER_FLOOR)
}

/// Bytes moved over the last few seconds.
#[derive(Debug, Default)]
pub(crate) struct RateWindow {
    samples: VecDeque<(u64, u64)>,
}

impl RateWindow {
    /// Records the running byte total at `now_ms`.
    pub(crate) fn record(&mut self, now_ms: u64, total: u64) {
        self.samples.push_back((now_ms, total));
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now_ms.saturating_sub(*at) > RATE_SPAN_MS)
            && self.samples.len() > 2
        {
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the recorded span (0 until two samples exist).
    pub(crate) fn rate(&self) -> u64 {
        let (Some((first_at, first)), Some((last_at, last))) =
            (self.samples.front(), self.samples.back())
        else {
            return 0;
        };
        let span = last_at.saturating_sub(*first_at);
        if span == 0 {
            return 0;
        }
        last.saturating_sub(*first).saturating_mul(1000) / span
    }
}

/// Packet size for small files from the measured packet rate.
#[derive(Debug, Default)]
pub(crate) struct BatchSizer {
    rate_bps: Option<f64>,
}

impl BatchSizer {
    /// A finished packet of `bytes` that took `elapsed_ms`.
    pub(crate) fn record(&mut self, bytes: u64, elapsed_ms: u64) {
        if bytes == 0 {
            return;
        }
        let rate = bytes as f64 * 1000.0 / elapsed_ms.max(1) as f64;
        self.rate_bps = Some(match self.rate_bps {
            Some(previous) => previous * 0.7 + rate * 0.3,
            None => rate,
        });
    }

    /// Bytes one packet should carry, within the backend's bound.
    pub(crate) fn target_bytes(&self, max_bytes: u64) -> u64 {
        let wanted = self
            .rate_bps
            .map(|rate| (rate * BATCH_TARGET_MS / 1000.0) as u64)
            .unwrap_or(MIN_BATCH_BYTES);
        wanted.clamp(MIN_BATCH_BYTES.min(max_bytes), max_bytes.max(1))
    }

    /// Files up to this size travel in packets.
    pub(crate) fn small_file_limit(&self, max_bytes: u64) -> u64 {
        self.target_bytes(max_bytes) / FILES_PER_BATCH_SHARE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_policy_classifies_failures() {
        assert!(is_transient(ErrorKind::ConnectionReset, false));
        assert!(is_transient(ErrorKind::Other, true));
        assert!(!is_transient(ErrorKind::NotFound, false));
        assert!(ends_job_at_target(ErrorKind::StorageFull));
        assert!(ends_job_at_target(ErrorKind::QuotaExceeded));
        assert!(!ends_job_at_target(ErrorKind::TimedOut));
        assert!(connection_failure(ErrorKind::NetworkUnreachable, false));
        assert!(!connection_failure(ErrorKind::InvalidData, false));
    }

    #[test]
    fn transfer_engine_task_policy_back_pressure_waits_within_patience() {
        assert!(is_back_pressure(ErrorKind::Other, true), "congestion");
        assert!(
            !is_back_pressure(ErrorKind::TimedOut, true),
            "a timeout may be a dead link"
        );
        assert!(!is_back_pressure(ErrorKind::ConnectionReset, false));
        assert!(overload_keeps_waiting(
            OVERLOAD_PATIENCE - Duration::from_secs(1)
        ));
        assert!(!overload_keeps_waiting(OVERLOAD_PATIENCE));
        assert_eq!(
            OVERLOAD_PATIENCE,
            Duration::from_secs(300),
            "shared with sync"
        );
        assert!(surely_not_done(ErrorKind::Other, true), "refused");
        assert!(surely_not_done(ErrorKind::NotConnected, false), "not sent");
        assert!(
            !surely_not_done(ErrorKind::TimedOut, true),
            "the answer may be lost"
        );
        assert!(!surely_not_done(ErrorKind::ConnectionReset, false));
    }

    #[test]
    fn transfer_engine_task_policy_listing_times_compare_at_their_grain() {
        assert!(!listed_time_differs(1_700_000_000_123, 1_700_000_000_123));
        assert!(!listed_time_differs(1_700_000_000_000, 1_700_000_000_400));
        assert!(listed_time_differs(1_700_000_001_500, 1_700_000_003_000));
        assert!(
            !listed_time_differs(1_699_999_980_000, 1_700_000_003_000),
            "minute listing"
        );
        assert!(!listed_time_differs(0, 1_700_000_003_000), "unknown time");
    }

    #[test]
    fn transfer_engine_task_policy_backoff_follows_the_peer_and_jitters() {
        assert_eq!(
            retry_delay(Some(Duration::from_secs(3)), 0.9),
            Duration::from_secs(3)
        );
        assert_eq!(
            retry_delay(Some(Duration::from_secs(600)), 0.0),
            RETRY_AFTER_MAX
        );
        assert_eq!(retry_delay(None, 0.0), RETRY_BASE / 2);
        assert_eq!(retry_delay(None, 1.0), RETRY_BASE.mul_f64(1.5));
    }

    #[test]
    fn transfer_engine_task_policy_breaker_scales_with_parallelism() {
        let mut breaker = Breaker::default();
        for _ in 0..7 {
            assert!(!breaker.failure(1));
        }
        assert!(breaker.failure(1), "the floor trips after eight");
        breaker.success();
        for _ in 0..19 {
            assert!(!breaker.failure(10));
        }
        assert!(breaker.failure(10), "twice the running operations");
    }

    #[test]
    fn transfer_engine_task_policy_rate_and_batch_sizes() {
        let mut window = RateWindow::default();
        window.record(0, 0);
        window.record(1_000, 1_000_000);
        window.record(2_000, 2_000_000);
        assert_eq!(window.rate(), 1_000_000);
        window.record(10_000, 2_000_000);
        assert_eq!(window.rate(), 0, "old samples leave the window");

        let mut sizer = BatchSizer::default();
        let max = 16 * 1024 * 1024;
        assert_eq!(sizer.target_bytes(max), MIN_BATCH_BYTES);
        sizer.record(40_000_000, 1_000);
        assert_eq!(sizer.target_bytes(max), 10_000_000);
        assert_eq!(sizer.small_file_limit(max), 1_250_000);
        sizer.record(4_000_000_000, 1_000);
        assert_eq!(
            sizer.target_bytes(max),
            max,
            "never above the backend bound"
        );
    }
}
