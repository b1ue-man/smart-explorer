//! Drive's answers to overload and exhausted quotas, and the waits of the
//! retry loops, which may hide a rate limit only for a bounded time (plan K13).
use std::fmt;
use std::io;
use std::time::Duration;

/// Longest a retry loop may keep a rate limit from its caller (the plan's
/// "Frist", K13). The old six-attempt read schedule waited 12.4 s in total
/// (0.4 + 0.8 + 1.6 + 3.2 + 6.4 s); browsing, scans and mounts, which do not
/// react to congestion themselves, rely on that tolerance. With the random
/// share of up to half a step (below) the same five waits take at most 18.6 s.
pub(super) const CONGESTION_HIDE_LIMIT: Duration = Duration::from_millis(18_600);

/// How Drive answered with an error status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusClass {
    /// Slow down: 429, 503 or a 403 rate-limit reason (ref §2).
    Congestion,
    /// Retrying cannot help: Drive storage full or the daily quota spent.
    Permanent(io::ErrorKind),
    /// A server fault that an idempotent read may try again.
    ServerFault,
    Other,
}

/// Classify a Drive error status by its code and (JSON or text) body.
pub(super) fn classify(code: u16, body: &str) -> StatusClass {
    match code {
        429 | 503 => StatusClass::Congestion,
        500 | 502 | 504 => StatusClass::ServerFault,
        403 if body.contains("storageQuotaExceeded") => {
            StatusClass::Permanent(io::ErrorKind::StorageFull)
        }
        403 if body.contains("dailyLimitExceeded") => {
            StatusClass::Permanent(io::ErrorKind::QuotaExceeded)
        }
        // rateLimitExceeded, userRateLimitExceeded, sharingRateLimitExceeded
        // and the per-minute quotaExceeded ask for a lower request rate.
        403 if body.contains("ateLimitExceeded") || body.contains("quotaExceeded") => {
            StatusClass::Congestion
        }
        _ => StatusClass::Other,
    }
}

/// An error status that is neither congestion nor carries its own kind; the
/// code stays inspectable (for example 404 after an exact-ID lookup).
#[derive(Debug)]
pub(super) struct DriveStatus {
    code: u16,
    message: String,
}

impl fmt::Display for DriveStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DriveStatus {}

/// The HTTP status behind a Drive error, if it was one (congestion excepted).
pub(super) fn http_status(error: &io::Error) -> Option<u16> {
    error
        .get_ref()?
        .downcast_ref::<DriveStatus>()
        .map(|status| status.code)
}

/// Turn a Drive error response into an `io::Error`: congestion as
/// `vfs::congestion_error`, storage/daily quota with their permanent kinds,
/// everything else readable as "HTTP 403: … (reason)".
pub(super) fn status_error(code: u16, retry_after: Option<Duration>, body: String) -> io::Error {
    let class = classify(code, &body);
    let message = format!("HTTP {}: {}", code, readable(body));
    match class {
        StatusClass::Congestion => crate::vfs::congestion_error(message, retry_after),
        StatusClass::Permanent(kind) => io::Error::new(kind, DriveStatus { code, message }),
        StatusClass::ServerFault | StatusClass::Other => {
            io::Error::other(DriveStatus { code, message })
        }
    }
}

/// Read the complete error body (so the socket returns to the pool) and
/// classify it.
pub(super) fn response_error(code: u16, response: ureq::Response) -> io::Error {
    let retry_after = retry_after(&response);
    let body = response.into_string().unwrap_or_default();
    status_error(code, retry_after, body)
}

/// `Retry-After` in delta-seconds; an HTTP date is left to the caller's own
/// backoff.
pub(super) fn retry_after(response: &ureq::Response) -> Option<Duration> {
    response
        .header("Retry-After")?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// A transport failure keeps the kind of its socket error (timeout, reset,
/// refused …) so transfers can tell transient network trouble apart.
pub(super) fn transport_error(error: ureq::Error) -> io::Error {
    let kind = std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<io::Error>())
        .map_or(io::ErrorKind::Other, io::Error::kind);
    io::Error::new(kind, error.to_string())
}

/// Drive's JSON error (`{"error":{"message":…,"errors":[{"reason":…}]}}`) as
/// "message (reason)"; any other body verbatim.
fn readable(body: String) -> String {
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value["error"]["message"].as_str().map(|message| {
                let reason = value["error"]["errors"][0]["reason"].as_str().unwrap_or("");
                if reason.is_empty() {
                    message.to_string()
                } else {
                    format!("{message} ({reason})")
                }
            })
        })
        .unwrap_or(body)
}

/// Exponential waits with a random share, so threads that hit the same limit
/// at once do not retry in lockstep (Drive's limits guide asks for jitter,
/// ref §2); rate limits count against `CONGESTION_HIDE_LIMIT`.
pub(super) struct Backoff {
    step: Duration,
    cap: Duration,
    hidden: Duration,
}

impl Backoff {
    pub(super) fn new(first: Duration, cap: Duration) -> Self {
        Self {
            step: first,
            cap,
            hidden: Duration::ZERO,
        }
    }

    /// Wait before the next attempt after `error`; `None` when a rate limit
    /// would stay hidden from the caller longer than the hide limit.
    pub(super) fn wait_after(&mut self, error: &io::Error) -> Option<Duration> {
        let step = self.step;
        self.step = step.saturating_mul(2).min(self.cap);
        let computed = jittered(step);
        let Some(congestion) = crate::vfs::congestion_of(error) else {
            return Some(computed);
        };
        let wait = congestion
            .retry_after
            .map_or(computed, |hint| hint.max(computed));
        let hidden = self.hidden.saturating_add(wait);
        if hidden > CONGESTION_HIDE_LIMIT {
            return None;
        }
        self.hidden = hidden;
        Some(wait)
    }
}

/// Sleep for a backoff wait. Unit tests count waits against the hide limit
/// without spending them, like the resumable retry delays always did.
pub(super) fn pause(wait: Duration) {
    if !cfg!(test) {
        std::thread::sleep(wait);
    }
}

/// `step` plus a random share of up to half of it.
pub(super) fn jittered(step: Duration) -> Duration {
    step + random_share(step / 2)
}

/// Uniform random duration below `max` (zero if no randomness is available).
fn random_share(max: Duration) -> Duration {
    let limit = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX);
    if limit == 0 {
        return Duration::ZERO;
    }
    let mut bytes = [0u8; 8];
    if getrandom::getrandom(&mut bytes).is_err() {
        return Duration::ZERO;
    }
    Duration::from_nanos(u64::from_le_bytes(bytes) % limit)
}

/// Random multipart boundary (128 bits, far below RFC 2046's 70 characters).
pub(super) fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        io::Error::other(format!("Zufallswert für Drive-Upload fehlt: {error}"))
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate_limited(retry_after: Option<Duration>) -> io::Error {
        status_error(
            403,
            retry_after,
            r#"{"error":{"message":"Rate Limit Exceeded","errors":[{"reason":"userRateLimitExceeded"}]}}"#
                .into(),
        )
    }

    #[test]
    fn transfer_engine_task_drive_statuses_map_to_congestion_or_permanent_kinds() {
        assert_eq!(classify(429, ""), StatusClass::Congestion);
        assert_eq!(classify(503, ""), StatusClass::Congestion);
        assert_eq!(classify(403, "rateLimitExceeded"), StatusClass::Congestion);
        assert_eq!(classify(403, "quotaExceeded"), StatusClass::Congestion);
        assert_eq!(
            classify(403, r#"{"reason":"storageQuotaExceeded"}"#),
            StatusClass::Permanent(io::ErrorKind::StorageFull)
        );
        assert_eq!(
            classify(403, "dailyLimitExceeded"),
            StatusClass::Permanent(io::ErrorKind::QuotaExceeded)
        );
        assert_eq!(classify(500, ""), StatusClass::ServerFault);
        assert_eq!(
            classify(403, "insufficientFilePermissions"),
            StatusClass::Other
        );

        let congested = rate_limited(Some(Duration::from_secs(7)));
        let congestion = crate::vfs::congestion_of(&congested).unwrap();
        assert_eq!(congestion.retry_after, Some(Duration::from_secs(7)));
        assert_eq!(
            congested.to_string(),
            "HTTP 403: Rate Limit Exceeded (userRateLimitExceeded)"
        );
        let full = status_error(403, None, "storageQuotaExceeded".into());
        assert_eq!(full.kind(), io::ErrorKind::StorageFull);
        assert!(crate::vfs::congestion_of(&full).is_none());
        let missing = status_error(404, None, "{}".into());
        assert_eq!(http_status(&missing), Some(404));
        assert_eq!(missing.to_string(), "HTTP 404: {}");
    }

    #[test]
    fn transfer_engine_task_drive_backoff_hides_rate_limits_only_within_the_limit() {
        // A server hint beyond the hide limit reaches the caller at once.
        let mut backoff = Backoff::new(Duration::from_millis(400), Duration::from_secs(16));
        assert!(backoff
            .wait_after(&rate_limited(Some(Duration::from_secs(30))))
            .is_none());

        // Without a hint the old five waits fit, a further one does not.
        let mut backoff = Backoff::new(Duration::from_millis(400), Duration::from_secs(16));
        let error = rate_limited(None);
        let mut total = Duration::ZERO;
        for step in [400u64, 800, 1600, 3200, 6400] {
            let wait = backoff.wait_after(&error).unwrap();
            assert!(wait >= Duration::from_millis(step));
            assert!(wait < Duration::from_millis(step + step / 2));
            total += wait;
        }
        assert!(total <= CONGESTION_HIDE_LIMIT);
        assert!(backoff.wait_after(&error).is_none());

        // Ordinary transient failures are bounded by the caller's attempts.
        let mut backoff = Backoff::new(Duration::from_millis(400), Duration::from_secs(16));
        let reset = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        for _ in 0..20 {
            assert!(backoff.wait_after(&reset).unwrap() <= Duration::from_secs(24));
        }
    }
}
