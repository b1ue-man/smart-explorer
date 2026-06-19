use crate::vfs::VfsResult;
use std::io;
use std::time::Duration;

pub(super) const API: &str = "https://www.googleapis.com/drive/v3";
pub(super) const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
pub(super) const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

pub(super) fn err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// Export MIME type for a Google-Docs editors file (None = a normal binary file
/// that downloads directly via alt=media).
pub(super) fn export_format(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/vnd.google-apps.document" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        "application/vnd.google-apps.spreadsheet" => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
        "application/vnd.google-apps.presentation" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        "application/vnd.google-apps.drawing" => "image/png",
        m if m.starts_with("application/vnd.google-apps.") && m != FOLDER_MIME => "application/pdf",
        _ => return None,
    })
}

/// File extension matching `export_format`.
pub(super) fn export_ext(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/vnd.google-apps.document" => "docx",
        "application/vnd.google-apps.spreadsheet" => "xlsx",
        "application/vnd.google-apps.presentation" => "pptx",
        "application/vnd.google-apps.drawing" => "png",
        m if m.starts_with("application/vnd.google-apps.") && m != FOLDER_MIME => "pdf",
        _ => return None,
    })
}

pub(super) fn not_found(p: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("nicht gefunden: {}", p))
}

/// Turn a Drive API error response into a readable io::Error (Drive returns
/// `{"error":{"code":403,"message":"...","errors":[{"reason":"..."}]}}`), so
/// the user sees e.g. "HTTP 403: ... (accessNotConfigured)" instead of
/// "status 403".
fn drive_err(code: u16, body: String) -> io::Error {
    let msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v["error"]["message"].as_str().map(|m| {
                let reason = v["error"]["errors"][0]["reason"].as_str().unwrap_or("");
                if reason.is_empty() {
                    m.to_string()
                } else {
                    format!("{} ({})", m, reason)
                }
            })
        })
        .unwrap_or(body);
    io::Error::new(io::ErrorKind::Other, format!("HTTP {}: {}", code, msg))
}

/// Drive returns 429 / 5xx on transient overload and 403 with a
/// `rateLimitExceeded`/`userRateLimitExceeded`/`quotaExceeded` reason when a
/// user runs many requests at once. Those are safe to retry with backoff;
/// everything else is a hard error.
fn is_rate_limited(code: u16, body: &str) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504)
        || (code == 403 && (body.contains("ateLimitExceeded") || body.contains("uotaExceeded")))
}

/// Execute a Drive request, returning the streaming response. Retries transient
/// failures (rate-limit / 5xx / transport) with exponential backoff so the
/// parallel sync engine can drive high concurrency without falling over. The
/// closure rebuilds the request each attempt (ureq requests aren't reusable).
pub(super) fn open_stream<F>(f: F) -> VfsResult<ureq::Response>
where
    F: Fn() -> Result<ureq::Response, ureq::Error>,
{
    let mut delay = Duration::from_millis(400);
    let mut last: Option<io::Error> = None;
    for attempt in 0..6 {
        match f() {
            Ok(resp) => return Ok(resp),
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if attempt < 5 && is_rate_limited(code, &body) {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_secs(16));
                    last = Some(drive_err(code, body));
                    continue;
                }
                return Err(drive_err(code, body));
            }
            Err(e) => {
                if attempt < 5 {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_secs(16));
                    last = Some(err(e));
                    continue;
                }
                return Err(err(e));
            }
        }
    }
    Err(last.unwrap_or_else(|| err("retry exhausted")))
}

/// `open_stream` + read the whole body to a string (for JSON endpoints).
pub(super) fn send_retry<F>(f: F) -> VfsResult<String>
where
    F: Fn() -> Result<ureq::Response, ureq::Error>,
{
    open_stream(f)?.into_string().map_err(err)
}

/// Parse a (possibly empty) JSON body.
pub(super) fn parse_json(s: String) -> VfsResult<serde_json::Value> {
    if s.trim().is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        serde_json::from_str(&s).map_err(err)
    }
}
