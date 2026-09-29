use super::overload::{
    classify, pause, response_error, retry_after, status_error, transport_error, Backoff,
    StatusClass,
};
use crate::vfs::VfsResult;
use std::io::{self, Read};
use std::time::Duration;

pub(super) const API: &str = "https://www.googleapis.com/drive/v3";
pub(super) const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
pub(super) const DRIVE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const DRIVE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const RETRY_ATTEMPTS: usize = 6;
const RETRY_INITIAL_DELAY: Duration = Duration::from_millis(400);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(16);
/// Status and upload-session answers carry no content worth reading; a body
/// beyond this is unusual and cheaper to drop together with its socket.
const DRAIN_LIMIT: u64 = 64 * 1024;

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

/// A failed read as `io::Error`, and whether the same idempotent GET may run
/// again: congestion (429, 503, 403 rate limits), other 5xx and transport
/// failures. Storage/daily quota and every other status end at once.
fn failed_read(error: ureq::Error) -> (io::Error, bool) {
    match error {
        ureq::Error::Status(code, response) => {
            let hint = retry_after(&response);
            let body = response.into_string().unwrap_or_default();
            let again = matches!(
                classify(code, &body),
                StatusClass::Congestion | StatusClass::ServerFault
            );
            (status_error(code, hint, body), again)
        }
        transport => (transport_error(transport), true),
    }
}

/// Whether to retry after `error`, waiting first. Rate limits stay hidden only
/// within the hide limit; afterwards the caller gets the congestion itself.
fn retry_read(attempt: usize, backoff: &mut Backoff, error: &io::Error, again: bool) -> bool {
    if !again || attempt >= RETRY_ATTEMPTS {
        return false;
    }
    match backoff.wait_after(error) {
        Some(wait) => {
            pause(wait);
            true
        }
        None => false,
    }
}

/// Execute a Drive request, returning the streaming response. Retries transient
/// failures (rate-limit / 5xx / transport) with jittered exponential backoff so
/// the parallel sync engine can drive high concurrency without falling over.
/// The closure rebuilds the request each attempt (ureq requests aren't reusable).
pub(super) fn open_stream<F>(f: F) -> VfsResult<ureq::Response>
where
    F: Fn() -> DriveRequestResult,
{
    let mut backoff = Backoff::new(RETRY_INITIAL_DELAY, RETRY_MAX_DELAY);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let (error, again) = match f() {
            Ok(response) => return Ok(response),
            Err(error) => failed_read(*error),
        };
        if !retry_read(attempt, &mut backoff, &error, again) {
            return Err(error);
        }
    }
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
                let error = response_error(code, response);
                if (500..=599).contains(&code) {
                    // A gateway or application server can commit a mutation
                    // and still return 5xx while producing its response. Never
                    // replay it; let the caller reconcile the exact resource
                    // ID and expected postcondition just like ACK loss.
                    Err(MutationRequestError::Ambiguous(error))
                } else {
                    // Refused before running (a 429/403 rate limit arrives as
                    // typed congestion, never retried here).
                    Err(MutationRequestError::Definite(error))
                }
            }
            error => Err(MutationRequestError::Ambiguous(transport_error(error))),
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
    let mut backoff = Backoff::new(RETRY_INITIAL_DELAY, RETRY_MAX_DELAY);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let (error, again) = match f() {
            Ok(response) => match response.into_string() {
                Ok(body) => return Ok(body),
                Err(error) => (error, true),
            },
            Err(error) => failed_read(*error),
        };
        if !retry_read(attempt, &mut backoff, &error, again) {
            return Err(error);
        }
    }
}

/// Read what is left of a response body so its socket returns to the pool.
pub(super) fn drain(response: ureq::Response) {
    let mut rest = response.into_reader().take(DRAIN_LIMIT);
    let _ = io::copy(&mut rest, &mut io::sink());
}

/// Parse a (possibly empty) JSON body.
pub(super) fn parse_json(s: String) -> VfsResult<serde_json::Value> {
    if s.trim().is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        serde_json::from_str(&s).map_err(err)
    }
}
