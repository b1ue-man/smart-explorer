//! HTTP answers every WebDAV request can get, whatever it asked for.
//! Overload (429 Too Many Requests, 503 Service Unavailable) is congestion
//! carrying the server's Retry-After (RFC 9110 §10.2.3: seconds or an HTTP
//! date), so a transfer backs off instead of failing (plan K13); the request
//! is not repeated here. 507 Insufficient Storage (RFC 4918 §11.5) is a full
//! server, a lasting condition.
use super::multistatus::parse_http_date_ms;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Overload or a full server; `None` for every other answer.
pub(super) fn overload_or_full(error: &ureq::Error) -> Option<io::Error> {
    match error {
        ureq::Error::Status(429 | 503, response) => {
            let retry_after = response.header("Retry-After").and_then(retry_after);
            Some(crate::vfs::congestion_error(
                format!("WebDAV-Server ist überlastet: {error}"),
                retry_after,
            ))
        }
        ureq::Error::Status(507, _) => Some(io::Error::new(
            io::ErrorKind::StorageFull,
            format!("Kein Speicherplatz mehr auf dem WebDAV-Server: {error}"),
        )),
        _ => None,
    }
}

/// A Retry-After value: delay seconds or an HTTP date (a past date: now).
pub(super) fn retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = parse_http_date_ms(value)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now = i64::try_from(now.as_millis()).ok()?;
    Some(Duration::from_millis(
        u64::try_from(at.saturating_sub(now)).unwrap_or(0),
    ))
}

/// The first byte a `Content-Range: bytes first-last/length` answer carries.
pub(super) fn range_start(value: &str) -> Option<u64> {
    let range = value.trim().strip_prefix("bytes")?.trim_start();
    range.split('-').next()?.trim().parse().ok()
}
