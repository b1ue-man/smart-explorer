use super::api::drain;
use super::overload::{jittered, pause, Backoff};
pub(super) use super::resumable_session::Bearer;
use super::resumable_session::{request_failure, send_authenticated, RequestFailure, Session};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::time::Duration;

pub(super) const CHUNK_SIZE: usize = 8 * 1024 * 1024;
const MAX_RETRIES: usize = 6;
const MAX_NO_PROGRESS: usize = 6;
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// First wait after a failed chunk request, then doubling: the former
/// schedule (250 ms << failures, from 500 ms up to 8 s), now with jitter.
const FIRST_RETRY: Duration = Duration::from_millis(500);
const MAX_RETRY: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Completion {
    Confirmed,
    VerifyExpected,
}

/// The upload's bytes by offset: a disk spool, or the chunk a streaming writer
/// still holds in memory. Bodies stream from there, so a chunk request needs
/// no copy of its bytes (and no unreserved buffer, plan K7).
pub(super) trait ChunkSource {
    /// Exactly `len` bytes of the upload starting at `offset`.
    fn body(&mut self, offset: u64, len: usize) -> io::Result<Box<dyn Read + '_>>;
}

/// Where `Resumable::send_until` stopped.
pub(super) enum Sent {
    /// The server keeps every byte below this offset.
    Confirmed(u64),
    Complete(Completion),
}

/// One resumable upload session and the bytes the server confirmed. Content
/// requests are never replayed blindly: after a failure the session status
/// tells which bytes arrived, and only the rest is sent again.
pub(super) struct Resumable {
    session: Session,
    total: u64,
    expected_id: String,
    confirmed: u64,
    sent: u64,
}

impl Resumable {
    pub(super) fn new(
        agent: ureq::Agent,
        location: &str,
        total: u64,
        expected_id: &str,
    ) -> io::Result<Self> {
        Ok(Self {
            session: Session::new(agent, location)?,
            total,
            expected_id: expected_id.to_string(),
            confirmed: 0,
            sent: 0,
        })
    }

    /// Send the bytes from the confirmed offset up to `limit` (the whole rest
    /// when `limit` is the total) in requests of at most `max_chunk` bytes,
    /// resending whatever the server did not keep, until it confirmed `limit`
    /// or completed the upload.
    pub(super) fn send_until(
        &mut self,
        source: &mut dyn ChunkSource,
        limit: u64,
        max_chunk: usize,
        bearer: &mut Bearer<'_>,
    ) -> io::Result<Sent> {
        let mut offset = self.confirmed;
        let mut failures = 0usize;
        let mut no_progress = 0usize;
        let mut query_status = false;
        let mut completion_possible = false;
        let mut backoff = Backoff::new(FIRST_RETRY, MAX_RETRY);
        loop {
            if !query_status && offset >= limit {
                // Nothing left below `limit` (the caller asked for bytes that
                // are already confirmed).
                return Ok(if limit >= self.total {
                    Sent::Complete(Completion::VerifyExpected)
                } else {
                    Sent::Confirmed(offset)
                });
            }
            let (result, submitted) = if query_status {
                let result = send_authenticated(bearer, |auth| {
                    self.session
                        .status_request(auth, self.total)
                        .send_bytes(&[])
                })?;
                (result, self.sent)
            } else {
                let wanted = (limit - offset).min(max_chunk as u64) as usize;
                // A source that no longer holds these bytes fails here, not
                // as a transport error that would be retried.
                drop(source.body(offset, wanted)?);
                let end = offset + wanted as u64;
                completion_possible = end == self.total;
                let result = send_authenticated(bearer, |auth| {
                    let body = source.body(offset, wanted).map_err(ureq::Error::from)?;
                    // The explicit length keeps ureq from chunking the body.
                    self.session
                        .content_request(auth)
                        .set("Content-Length", &wanted.to_string())
                        .set(
                            "Content-Range",
                            &format!("bytes {offset}-{}/{}", end.saturating_sub(1), self.total),
                        )
                        .send(body)
                })?;
                self.sent = self.sent.max(end);
                (result, end)
            };

            match result {
                Ok(response) => {
                    failures = 0;
                    match classify(
                        response,
                        &mut self.session,
                        self.total,
                        self.confirmed,
                        Some(submitted),
                        &self.expected_id,
                    )? {
                        UploadStatus::Complete(done) => return Ok(Sent::Complete(done)),
                        UploadStatus::Offset(next) => {
                            completion_possible = false;
                            if next <= self.confirmed {
                                wait_for_progress(
                                    &mut no_progress,
                                    "Drive upload made no progress",
                                )?;
                            } else {
                                self.confirmed = next;
                                no_progress = 0;
                            }
                            offset = next;
                            query_status = false;
                            if next >= limit {
                                return Ok(Sent::Confirmed(next));
                            }
                        }
                    }
                }
                Err(error) => match request_failure(error) {
                    RequestFailure::Transient(error) => {
                        failures += 1;
                        let wait = if failures > MAX_RETRIES {
                            None
                        } else {
                            backoff.wait_after(&error)
                        };
                        let Some(wait) = wait else {
                            return ambiguous_or_error(completion_possible, error)
                                .map(Sent::Complete);
                        };
                        pause(wait);
                        query_status = true;
                    }
                    RequestFailure::Hard(_) if query_status && completion_possible => {
                        return Ok(Sent::Complete(Completion::VerifyExpected));
                    }
                    RequestFailure::Hard(error) => return Err(error),
                },
            }
        }
    }
}

struct SpoolSource<'a> {
    spool: &'a mut File,
}

impl ChunkSource for SpoolSource<'_> {
    fn body(&mut self, offset: u64, len: usize) -> io::Result<Box<dyn Read + '_>> {
        self.spool.seek(SeekFrom::Start(offset))?;
        Ok(Box::new(self.spool.by_ref().take(len as u64)))
    }
}

/// Upload a complete spool through one session in chunks of `CHUNK_SIZE`.
// `ureq::Error` remains intact so status responses can be retried and classified
// with their response bodies without changing the caller-visible error behavior.
#[allow(clippy::result_large_err)]
pub(super) fn upload<GetBearer, RefreshBearer>(
    agent: &ureq::Agent,
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
    let mut bearer = Bearer {
        get: &mut get_bearer,
        refresh: &mut refresh_bearer,
    };
    let mut upload = Resumable::new(agent.clone(), location, total, expected_id)?;
    if total == 0 {
        return upload_empty(&mut upload.session, expected_id, &mut bearer);
    }
    send_spool(&mut upload, spool, &mut bearer)
}

/// Send the whole (non-empty) spool through `upload` in `CHUNK_SIZE` requests.
pub(super) fn send_spool(
    upload: &mut Resumable,
    spool: &mut File,
    bearer: &mut Bearer<'_>,
) -> io::Result<Completion> {
    let total = upload.total;
    let mut source = SpoolSource { spool };
    match upload.send_until(&mut source, total, CHUNK_SIZE, bearer)? {
        Sent::Complete(done) => Ok(done),
        // Every byte confirmed without a final answer: verify by exact ID.
        Sent::Confirmed(_) => Ok(Completion::VerifyExpected),
    }
}

#[allow(clippy::result_large_err)]
fn upload_empty(
    session: &mut Session,
    expected_id: &str,
    bearer: &mut Bearer<'_>,
) -> io::Result<Completion> {
    let mut failures = 0usize;
    let mut no_progress = 0usize;
    let mut query_status = false;
    let mut completion_possible = false;
    let mut backoff = Backoff::new(FIRST_RETRY, MAX_RETRY);
    loop {
        let result = if query_status {
            send_authenticated(bearer, |auth| {
                session.status_request(auth, 0).send_bytes(&[])
            })?
        } else {
            completion_possible = true;
            send_authenticated(bearer, |auth| {
                session
                    .content_request(auth)
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
                    let wait = if failures > MAX_RETRIES {
                        None
                    } else {
                        backoff.wait_after(&error)
                    };
                    let Some(wait) = wait else {
                        return ambiguous_or_error(completion_possible, error);
                    };
                    pause(wait);
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
            let next = confirmed_offset(&response, total, minimum, submitted_limit);
            drain(response);
            let next = next?;
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

fn ambiguous_or_error(possible: bool, error: io::Error) -> io::Result<Completion> {
    if possible {
        Ok(Completion::VerifyExpected)
    } else {
        Err(error)
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
    pause(retry_delay(*retries));
    Ok(())
}

fn retry_delay(failures: usize) -> Duration {
    jittered(Duration::from_millis(
        (250u64 << failures.min(5)).min(8_000),
    ))
}

#[cfg(test)]
#[path = "resumable_tests.rs"]
mod tests;
