use super::api::is_rate_limited;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::time::Duration;

const CHUNK_SIZE: usize = 8 * 1024 * 1024;
const MAX_RETRIES: usize = 6;
const MAX_NO_PROGRESS: usize = 6;
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Completion {
    Confirmed,
    VerifyExpected,
}

// `ureq::Error` remains intact so status responses can be retried and classified
// with their response bodies without changing the caller-visible error behavior.
#[allow(clippy::result_large_err)]
pub(super) fn upload<GetBearer, RefreshBearer>(
    location: &str,
    spool: &mut File,
    total: u64,
    expected_id: &str,
    mut get_bearer: GetBearer,
    mut refresh_bearer: RefreshBearer,
) -> io::Result<Completion>
where
    GetBearer: FnMut() -> io::Result<String>,
    RefreshBearer: FnMut() -> io::Result<String>,
{
    let mut session = Session::new(location)?;
    if total == 0 {
        return upload_empty(
            &mut session,
            expected_id,
            &mut get_bearer,
            &mut refresh_bearer,
        );
    }

    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut offset = 0u64;
    let mut high_water = 0u64;
    let mut failures = 0usize;
    let mut no_progress = 0usize;
    let mut query_status = false;
    let mut completion_possible = false;
    loop {
        let (result, submitted_limit) = if query_status {
            (
                send_authenticated(&mut get_bearer, &mut refresh_bearer, |bearer| {
                    session.status_request(bearer, total).send_bytes(&[])
                })?,
                None,
            )
        } else {
            spool.seek(SeekFrom::Start(offset))?;
            let wanted = (total - offset).min(buffer.len() as u64) as usize;
            spool.read_exact(&mut buffer[..wanted])?;
            let limit = offset + wanted as u64;
            completion_possible = limit == total;
            let result = send_authenticated(&mut get_bearer, &mut refresh_bearer, |bearer| {
                session
                    .content_request(bearer)
                    .set("Content-Length", &wanted.to_string())
                    .set(
                        "Content-Range",
                        &format!("bytes {offset}-{}/{total}", limit - 1),
                    )
                    .send_bytes(&buffer[..wanted])
            })?;
            (result, Some(limit))
        };

        match result {
            Ok(response) => {
                failures = 0;
                match classify(
                    response,
                    &mut session,
                    total,
                    high_water,
                    submitted_limit,
                    expected_id,
                )? {
                    UploadStatus::Complete(done) => return Ok(done),
                    UploadStatus::Offset(next) => {
                        completion_possible = false;
                        if next <= high_water {
                            wait_for_progress(&mut no_progress, "Drive upload made no progress")?;
                        } else {
                            high_water = next;
                            no_progress = 0;
                        }
                        offset = next;
                        query_status = false;
                    }
                }
            }
            Err(error) => match request_failure(error) {
                RequestFailure::Transient(error) => {
                    failures += 1;
                    if failures > MAX_RETRIES {
                        return ambiguous_or_error(completion_possible, error);
                    }
                    std::thread::sleep(retry_delay(failures));
                    query_status = true;
                }
                RequestFailure::Hard(_) if query_status && completion_possible => {
                    return Ok(Completion::VerifyExpected);
                }
                RequestFailure::Hard(error) => return Err(error),
            },
        }
    }
}

#[allow(clippy::result_large_err)]
fn upload_empty<GetBearer, RefreshBearer>(
    session: &mut Session,
    expected_id: &str,
    get_bearer: &mut GetBearer,
    refresh_bearer: &mut RefreshBearer,
) -> io::Result<Completion>
where
    GetBearer: FnMut() -> io::Result<String>,
    RefreshBearer: FnMut() -> io::Result<String>,
{
    let mut failures = 0usize;
    let mut no_progress = 0usize;
    let mut query_status = false;
    let mut completion_possible = false;
    loop {
        let result = if query_status {
            send_authenticated(get_bearer, refresh_bearer, |bearer| {
                session.status_request(bearer, 0).send_bytes(&[])
            })?
        } else {
            completion_possible = true;
            send_authenticated(get_bearer, refresh_bearer, |bearer| {
                session
                    .content_request(bearer)
                    .set("Content-Length", "0")
                    .send_bytes(&[])
            })?
        };
        match result {
            Ok(response) => {
                failures = 0;
                match classify(response, session, 0, 0, None, expected_id)? {
                    UploadStatus::Complete(done) => return Ok(done),
                    UploadStatus::Offset(_) => {
                        completion_possible = false;
                        wait_for_progress(&mut no_progress, "Drive empty upload made no progress")?;
                        query_status = false;
                    }
                }
            }
            Err(error) => match request_failure(error) {
                RequestFailure::Transient(error) => {
                    failures += 1;
                    if failures > MAX_RETRIES {
                        return ambiguous_or_error(completion_possible, error);
                    }
                    std::thread::sleep(retry_delay(failures));
                    query_status = true;
                }
                RequestFailure::Hard(_) if query_status && completion_possible => {
                    return Ok(Completion::VerifyExpected);
                }
                RequestFailure::Hard(error) => return Err(error),
            },
        }
    }
}

enum UploadStatus {
    Complete(Completion),
    Offset(u64),
}

fn classify(
    response: ureq::Response,
    session: &mut Session,
    total: u64,
    minimum: u64,
    submitted_limit: Option<u64>,
    expected_id: &str,
) -> io::Result<UploadStatus> {
    match response.status() {
        200 | 201 => completion(response, expected_id).map(UploadStatus::Complete),
        308 => {
            session.update_from(&response)?;
            let next = confirmed_offset(&response, total, minimum, submitted_limit)?;
            if total > 0 && next == total {
                Ok(UploadStatus::Complete(Completion::VerifyExpected))
            } else {
                Ok(UploadStatus::Offset(next))
            }
        }
        status => {
            let body = response.into_string().unwrap_or_default();
            Err(io::Error::other(format!(
                "Drive unexpected HTTP {status}: {body}"
            )))
        }
    }
}

fn completion(response: ureq::Response, expected_id: &str) -> io::Result<Completion> {
    let id = response
        .into_string()
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|json| json["id"].as_str().map(str::to_owned));
    match id {
        Some(id) if id == expected_id => Ok(Completion::Confirmed),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Drive upload completed with an unexpected file id",
        )),
        None => Ok(Completion::VerifyExpected),
    }
}

fn confirmed_offset(
    response: &ureq::Response,
    total: u64,
    minimum: u64,
    submitted_limit: Option<u64>,
) -> io::Result<u64> {
    let next = match response.header("Range") {
        None => 0,
        Some(range) => range
            .strip_prefix("bytes=0-")
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|end| end.checked_add(1))
            .ok_or_else(invalid_range)?,
    };
    if next < minimum || next > total || submitted_limit.is_some_and(|limit| next > limit) {
        return Err(invalid_range());
    }
    Ok(next)
}

struct Session {
    url: String,
    agent: ureq::Agent,
}

impl Session {
    fn new(location: &str) -> io::Result<Self> {
        validate_session_url(location)?;
        Ok(Self {
            url: location.to_string(),
            agent: ureq::AgentBuilder::new().redirects(0).build(),
        })
    }

    fn update_from(&mut self, response: &ureq::Response) -> io::Result<()> {
        let Some(location) = response.header("Location") else {
            return Ok(());
        };
        validate_session_url(location)?;
        self.url = location.to_string();
        Ok(())
    }

    fn content_request(&self, bearer: &str) -> ureq::Request {
        self.request(bearer)
    }

    fn status_request(&self, bearer: &str, total: u64) -> ureq::Request {
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

fn ambiguous_or_error(possible: bool, error: io::Error) -> io::Result<Completion> {
    if possible {
        Ok(Completion::VerifyExpected)
    } else {
        Err(error)
    }
}

fn send_authenticated<GetBearer, RefreshBearer, Build>(
    get_bearer: &mut GetBearer,
    refresh_bearer: &mut RefreshBearer,
    mut build: Build,
) -> io::Result<Result<ureq::Response, ureq::Error>>
where
    GetBearer: FnMut() -> io::Result<String>,
    RefreshBearer: FnMut() -> io::Result<String>,
    Build: FnMut(&str) -> Result<ureq::Response, ureq::Error>,
{
    let bearer = format!("Bearer {}", get_bearer()?);
    match build(&bearer) {
        Err(ureq::Error::Status(401, _)) => {
            let bearer = format!("Bearer {}", refresh_bearer()?);
            Ok(build(&bearer))
        }
        result => Ok(result),
    }
}

enum RequestFailure {
    Transient(io::Error),
    Hard(io::Error),
}

fn request_failure(error: ureq::Error) -> RequestFailure {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let error = io::Error::other(format!("Drive upload HTTP {code}: {body}"));
            if is_rate_limited(code, &body) {
                RequestFailure::Transient(error)
            } else {
                RequestFailure::Hard(error)
            }
        }
        ureq::Error::Transport(error) => {
            RequestFailure::Transient(io::Error::other(error.to_string()))
        }
    }
}

fn request_error(error: ureq::Error) -> io::Error {
    match request_failure(error) {
        RequestFailure::Transient(error) | RequestFailure::Hard(error) => error,
    }
}

fn invalid_range() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Drive resumable upload returned an invalid Range",
    )
}

fn wait_for_progress(retries: &mut usize, message: &'static str) -> io::Result<()> {
    *retries += 1;
    if *retries > MAX_NO_PROGRESS {
        return Err(io::Error::new(io::ErrorKind::TimedOut, message));
    }
    std::thread::sleep(retry_delay(*retries));
    Ok(())
}

fn retry_delay(failures: usize) -> Duration {
    if cfg!(test) {
        Duration::ZERO
    } else {
        Duration::from_millis((250u64 << failures.min(5)).min(8_000))
    }
}

#[cfg(test)]
#[path = "resumable_tests.rs"]
mod tests;
