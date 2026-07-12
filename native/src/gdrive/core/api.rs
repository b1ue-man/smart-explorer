use crate::vfs::VfsResult;
use std::io;
use std::time::Duration;

pub(super) const API: &str = "https://www.googleapis.com/drive/v3";
pub(super) const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
pub(super) const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
pub(super) const DRIVE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const DRIVE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const RETRY_ATTEMPTS: usize = 6;
const RETRY_INITIAL_DELAY: Duration = Duration::from_millis(400);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(16);

pub(super) type DriveRequestResult = Result<ureq::Response, Box<ureq::Error>>;

#[derive(Debug)]
pub(super) enum MutationRequestError {
    Definite(io::Error),
    Ambiguous(io::Error),
}

impl MutationRequestError {
    pub(super) fn into_io(self) -> io::Error {
        match self {
            Self::Definite(error) | Self::Ambiguous(error) => error,
        }
    }
}

pub(super) fn drive_request(result: Result<ureq::Response, ureq::Error>) -> DriveRequestResult {
    result.map_err(Box::new)
}

pub(super) fn err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
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
    io::Error::other(format!("HTTP {}: {}", code, msg))
}

/// Drive returns 429 / 5xx on transient overload and 403 with a
/// `rateLimitExceeded`/`userRateLimitExceeded`/`quotaExceeded` reason when a
/// user runs many requests at once. Those are safe to retry with backoff;
/// everything else is a hard error.
pub(super) fn is_rate_limited(code: u16, body: &str) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504)
        || (code == 403 && (body.contains("ateLimitExceeded") || body.contains("uotaExceeded")))
}

/// Execute a Drive request, returning the streaming response. Retries transient
/// failures (rate-limit / 5xx / transport) with exponential backoff so the
/// parallel sync engine can drive high concurrency without falling over. The
/// closure rebuilds the request each attempt (ureq requests aren't reusable).
pub(super) fn open_stream<F>(f: F) -> VfsResult<ureq::Response>
where
    F: Fn() -> DriveRequestResult,
{
    let mut delay = RETRY_INITIAL_DELAY;
    let mut last: Option<io::Error> = None;
    for attempt in 0..RETRY_ATTEMPTS {
        match f() {
            Ok(resp) => return Ok(resp),
            Err(e) => match *e {
                ureq::Error::Status(code, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    if attempt + 1 < RETRY_ATTEMPTS && is_rate_limited(code, &body) {
                        last = Some(drive_err(code, body));
                        sleep_before_retry(&mut delay);
                        continue;
                    }
                    return Err(drive_err(code, body));
                }
                e => {
                    if attempt + 1 < RETRY_ATTEMPTS {
                        last = Some(err(e));
                        sleep_before_retry(&mut delay);
                        continue;
                    }
                    return Err(err(e));
                }
            },
        }
    }
    Err(last.unwrap_or_else(|| err("retry exhausted")))
}

fn sleep_before_retry(delay: &mut Duration) {
    std::thread::sleep(*delay);
    *delay = (*delay * 2).min(RETRY_MAX_DELAY);
}

/// Execute one mutation request exactly once. A transport failure after send is
/// ambiguous and must be reconciled by the operation's exact resource ID. The
/// already-executed result makes accidental replay impossible in this helper.
pub(super) fn open_once(result: DriveRequestResult) -> VfsResult<ureq::Response> {
    mutation_once(result).map_err(MutationRequestError::into_io)
}

pub(super) fn mutation_once(
    result: DriveRequestResult,
) -> Result<ureq::Response, MutationRequestError> {
    match result {
        Ok(response) => Ok(response),
        Err(error) => match *error {
            ureq::Error::Status(code, response) => {
                let body = response.into_string().unwrap_or_default();
                let error = drive_err(code, body);
                if (500..=599).contains(&code) {
                    // A gateway or application server can commit a mutation
                    // and still return 5xx while producing its response. Never
                    // replay it; let the caller reconcile the exact resource
                    // ID and expected postcondition just like ACK loss.
                    Err(MutationRequestError::Ambiguous(error))
                } else {
                    Err(MutationRequestError::Definite(error))
                }
            }
            error => Err(MutationRequestError::Ambiguous(err(error))),
        },
    }
}

/// Rebuild a metadata GET and consume its complete JSON body within the same
/// bounded retry attempt. No response bytes escape this helper, so retrying a
/// dropped or timed-out body is safe. Mutations use `mutation_once` instead.
pub(super) fn send_retry<F>(f: F) -> VfsResult<String>
where
    F: Fn() -> DriveRequestResult,
{
    let mut delay = RETRY_INITIAL_DELAY;
    let mut last: Option<io::Error> = None;
    for attempt in 0..RETRY_ATTEMPTS {
        match f() {
            Ok(response) => match response.into_string() {
                Ok(body) => return Ok(body),
                Err(error) if attempt + 1 < RETRY_ATTEMPTS => {
                    last = Some(error);
                    sleep_before_retry(&mut delay);
                }
                Err(error) => return Err(error),
            },
            Err(error) => match *error {
                ureq::Error::Status(code, response) => {
                    let body = response.into_string().unwrap_or_default();
                    if attempt + 1 < RETRY_ATTEMPTS && is_rate_limited(code, &body) {
                        last = Some(drive_err(code, body));
                        sleep_before_retry(&mut delay);
                        continue;
                    }
                    return Err(drive_err(code, body));
                }
                error if attempt + 1 < RETRY_ATTEMPTS => {
                    last = Some(err(error));
                    sleep_before_retry(&mut delay);
                }
                error => return Err(err(error)),
            },
        }
    }
    Err(last.unwrap_or_else(|| err("metadata read retry exhausted")))
}

/// Parse a (possibly empty) JSON body.
pub(super) fn parse_json(s: String) -> VfsResult<serde_json::Value> {
    if s.trim().is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        serde_json::from_str(&s).map_err(err)
    }
}

pub(super) fn parse_generated_id(json: &serde_json::Value) -> VfsResult<String> {
    json["ids"]
        .as_array()
        .and_then(|ids| ids.first())
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive generateIds response has no usable id",
            )
        })
}
