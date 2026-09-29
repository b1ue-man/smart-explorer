//! The HTTP side of a resumable upload session: the trusted session URL,
//! authenticated chunk and status requests, and how a failed request is
//! classified (resumable after a status query, or final).
use super::api::drain;
use super::overload::{classify as classify_status, retry_after, transport_error, StatusClass};
use super::resumable::REQUEST_TIMEOUT;
use std::io;

/// Access tokens for chunk requests: the current one, and a forced refresh
/// for one retry after 401.
pub(super) struct Bearer<'a> {
    pub(super) get: &'a mut dyn FnMut() -> io::Result<String>,
    pub(super) refresh: &'a mut dyn FnMut() -> io::Result<String>,
}

/// One resumable upload session URL; the server may move it with any answer.
pub(super) struct Session {
    url: String,
    agent: ureq::Agent,
}

impl Session {
    pub(super) fn new(agent: ureq::Agent, location: &str) -> io::Result<Self> {
        validate_session_url(location)?;
        Ok(Self {
            url: location.to_string(),
            agent,
        })
    }

    pub(super) fn update_from(&mut self, response: &ureq::Response) -> io::Result<()> {
        let Some(location) = response.header("Location") else {
            return Ok(());
        };
        validate_session_url(location)?;
        self.url = location.to_string();
        Ok(())
    }

    pub(super) fn content_request(&self, bearer: &str) -> ureq::Request {
        self.request(bearer)
    }

    pub(super) fn status_request(&self, bearer: &str, total: u64) -> ureq::Request {
        self.request(bearer)
            .set("Content-Length", "0")
            .set("Content-Range", &format!("bytes */{total}"))
    }

    fn request(&self, bearer: &str) -> ureq::Request {
        self.agent
            .put(&self.url)
            .timeout(REQUEST_TIMEOUT)
            .set("Authorization", bearer)
    }
}

fn validate_session_url(location: &str) -> io::Result<()> {
    let request = ureq::put(location);
    let parsed = request.request_url().map_err(request_error)?;
    let url = parsed.as_url();
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Drive upload Location contains credentials or a fragment",
        ));
    }
    let scheme = parsed.scheme().to_ascii_lowercase();
    let host = parsed.host().to_ascii_lowercase();
    let google = scheme == "https"
        && url.port_or_known_default() == Some(443)
        && (host == "googleapis.com" || host.ends_with(".googleapis.com"));
    let test_local = cfg!(test)
        && scheme == "http"
        && matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
    if !google && !test_local {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Drive upload Location is not a trusted Google HTTPS URL",
        ));
    }
    Ok(())
}

/// Send once with the current token; after a 401 once more with a refreshed one.
pub(super) fn send_authenticated<Build>(
    bearer: &mut Bearer<'_>,
    mut build: Build,
) -> io::Result<Result<ureq::Response, ureq::Error>>
where
    Build: FnMut(&str) -> Result<ureq::Response, ureq::Error>,
{
    let token = format!("Bearer {}", (bearer.get)()?);
    match build(&token) {
        Err(ureq::Error::Status(401, rejected)) => {
            // Read the refusal so its socket serves the retry.
            drain(rejected);
            let token = format!("Bearer {}", (bearer.refresh)()?);
            Ok(build(&token))
        }
        result => Ok(result),
    }
}

pub(super) enum RequestFailure {
    Transient(io::Error),
    Hard(io::Error),
}

/// Rate limits (typed congestion) and server faults are resumed after a
/// status query; storage full, the daily quota and other statuses end the
/// upload.
pub(super) fn request_failure(error: ureq::Error) -> RequestFailure {
    match error {
        ureq::Error::Status(code, response) => {
            let hint = retry_after(&response);
            let body = response.into_string().unwrap_or_default();
            let class = classify_status(code, &body);
            let message = format!("Drive upload HTTP {code}: {body}");
            match class {
                StatusClass::Congestion => {
                    RequestFailure::Transient(crate::vfs::congestion_error(message, hint))
                }
                StatusClass::ServerFault => RequestFailure::Transient(io::Error::other(message)),
                StatusClass::Permanent(kind) => RequestFailure::Hard(io::Error::new(kind, message)),
                StatusClass::Other => RequestFailure::Hard(io::Error::other(message)),
            }
        }
        transport => RequestFailure::Transient(transport_error(transport)),
    }
}

fn request_error(error: ureq::Error) -> io::Error {
    match request_failure(error) {
        RequestFailure::Transient(error) | RequestFailure::Hard(error) => error,
    }
}
