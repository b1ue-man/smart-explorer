//! One resumable HTTP GET into a file or into memory (`core/decision.rs`).
//!
//! Every attempt has a limit to connect and a limit per read or write, never
//! one on the whole transfer: ureq's `timeout` bounds the whole request
//! including the body ("overall timeout", ureq 2.12.1), which cut off large
//! downloads on slow lines although bytes kept arriving (seen 2026-11-17 at
//! about 3 Mbit/s). A broken or stalled attempt continues with a range
//! request from the bytes kept; only attempts without a new byte for the
//! whole stall window end the download. Each attempt uses its own agent, so
//! no pooled socket without timeouts is reused.
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::time::{Duration, Instant};

use super::decision::{
    classify, if_range_validator, range_header, refusal, Answer, Attempts, Kept, Next, Patience,
    Refusal, Reply,
};

/// Time to open the connection (name lookup, TCP, TLS).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);
/// Time one read or write may wait without a byte moving.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK: usize = 64 * 1024;

#[cfg(not(any(test, feature = "test-support")))]
const PATIENCE: Patience = Patience::STANDARD;
/// Tests end a stalled download after the attempts alone: no two-minute
/// window, millisecond pauses (the decision itself is tested with
/// `Patience::STANDARD`, `decision_tests.rs`).
#[cfg(any(test, feature = "test-support"))]
const PATIENCE: Patience = Patience {
    idle_attempts: Patience::STANDARD.idle_attempts,
    stall_window: Duration::ZERO,
    first_pause: Duration::from_millis(1),
    max_pause: Duration::from_millis(20),
};

/// Where a download comes from.
pub(crate) struct Source<'a> {
    pub(crate) url: &'a str,
    pub(crate) user_agent: &'a str,
    pub(crate) https_only: bool,
    /// ureq's redirect setting (0: none followed).
    pub(crate) redirects: u32,
    /// Whether the address an answer came from (after redirects) may
    /// deliver the bytes.
    pub(crate) final_url: &'a dyn Fn(&str) -> bool,
}

/// The size limits of a download.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bound {
    /// The exact length a signature or a pin names: an answer naming another
    /// length and every byte beyond it fail the download.
    pub(crate) exact: Option<u64>,
    /// At most this many bytes while no answer names a length (the free
    /// space minus a reserve, r-lokal-4).
    pub(crate) ceiling: Option<u64>,
}

/// Where the bytes go. The caller hands it over empty.
pub(crate) trait Sink {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Drops every byte appended so far.
    fn restart(&mut self) -> io::Result<()>;
}

impl Sink for std::fs::File {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(bytes)
    }

    fn restart(&mut self) -> io::Result<()> {
        self.set_len(0)?;
        self.seek(SeekFrom::Start(0)).map(|_| ())
    }
}

impl Sink for Vec<u8> {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }

    fn restart(&mut self) -> io::Result<()> {
        self.clear();
        Ok(())
    }
}

/// The error of a cancelled download.
fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Download abgebrochen")
}

/// The message of a byte beyond the announced or signed length (the same
/// words as `updater::download_bound`).
const MORE_BYTES: &str = "the server sends more bytes than its announced or signed length";

/// How an attempt ended short of the whole resource.
enum Short {
    /// Another attempt may continue.
    Retry(io::Error),
    /// No attempt can change the outcome.
    Final(io::Error),
}

struct State {
    kept: u64,
    known: Option<u64>,
    validator: Option<String>,
}

/// Downloads `source` into `sink` within `bound`; returns the bytes written.
/// `progress` sees the bytes kept and the length, when known; `cancelled`
/// is asked before each read and during pauses (an `Interrupted` error then).
pub(crate) fn fetch(
    source: &Source<'_>,
    bound: Bound,
    sink: &mut dyn Sink,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancelled: &dyn Fn() -> bool,
) -> io::Result<u64> {
    let mut state = State {
        kept: 0,
        known: bound.exact,
        validator: None,
    };
    let mut attempts = Attempts::new(Instant::now());
    loop {
        if cancelled() {
            return Err(cancelled_error());
        }
        let error = match attempt(source, bound, &mut state, sink, progress, cancelled) {
            Ok(()) => return Ok(state.kept),
            Err(Short::Final(error)) => return Err(error),
            Err(Short::Retry(error)) => error,
        };
        match attempts.ended(state.kept, Instant::now(), &PATIENCE) {
            Next::Retry(pause) => wait(pause, cancelled)?,
            Next::Stalled => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error} (no new byte in {} attempts in a row)",
                        attempts.idle()
                    ),
                ))
            }
        }
    }
}

fn wait(pause: Duration, cancelled: &dyn Fn() -> bool) -> io::Result<()> {
    let until = Instant::now() + pause;
    loop {
        if cancelled() {
            return Err(cancelled_error());
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

/// Transport failures another attempt may not meet (a dropped line, a lost
/// name lookup, a cut answer); an invalid address or a refused scheme stays.
fn transport_may_pass(kind: ureq::ErrorKind) -> bool {
    matches!(
        kind,
        ureq::ErrorKind::Dns
            | ureq::ErrorKind::ConnectionFailed
            | ureq::ErrorKind::Io
            | ureq::ErrorKind::BadStatus
            | ureq::ErrorKind::BadHeader
            | ureq::ErrorKind::ProxyConnect
    )
}

fn restart(state: &mut State, sink: &mut dyn Sink, pinned: Option<u64>) -> Result<(), Short> {
    if state.kept > 0 {
        sink.restart().map_err(Short::Final)?;
    }
    state.kept = 0;
    state.known = pinned;
    state.validator = None;
    Ok(())
}

fn attempt(
    source: &Source<'_>,
    bound: Bound,
    state: &mut State,
    sink: &mut dyn Sink,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Short> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(IDLE_TIMEOUT)
        .timeout_write(IDLE_TIMEOUT)
        .https_only(source.https_only)
        .redirects(source.redirects)
        .user_agent(source.user_agent)
        .build();
    let mut request = agent.get(source.url);
    if let Some(range) = range_header(state.kept) {
        request = request.set("Range", &range);
        if let Some(tag) = state.validator.as_deref() {
            request = request.set("If-Range", tag);
        }
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(error) => {
            let decision = match &error {
                ureq::Error::Status(status, _) => refusal(*status),
                ureq::Error::Transport(_) if transport_may_pass(error.kind()) => Refusal::Wait,
                ureq::Error::Transport(_) => Refusal::Final,
            };
            // ureq names the address in its message.
            let error = io::Error::other(error.to_string());
            return Err(match decision {
                Refusal::Restart => {
                    restart(state, sink, bound.exact)?;
                    Short::Retry(error)
                }
                Refusal::Wait => Short::Retry(error),
                Refusal::Final => Short::Final(error),
            });
        }
    };
    if !(source.final_url)(response.get_url()) {
        return Err(Short::Final(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the download of {} ended at an address that is not allowed ({})",
                source.url,
                response.get_url()
            ),
        )));
    }
    let status = response.status();
    let answer = classify(
        Kept {
            offset: state.kept,
            pinned: bound.exact,
            known: state.known,
        },
        Reply {
            status,
            content_range: response.header("Content-Range"),
            content_length: response
                .header("Content-Length")
                .and_then(|value| value.trim().parse().ok()),
        },
    );
    let validator = if_range_validator(response.header("ETag")).map(str::to_owned);
    match answer {
        Answer::Continue { total } => {
            state.known = total;
            if state.validator.is_none() {
                state.validator = validator;
            }
        }
        Answer::Restart { total } => {
            restart(state, sink, bound.exact)?;
            state.known = total;
            state.validator = validator;
        }
        Answer::WrongLength { announced, pinned } => {
            let text = if announced > pinned {
                format!("{MORE_BYTES} ({announced} announced, {pinned} signed or pinned)")
            } else {
                format!("the server announces {announced} bytes, the signed or pinned length is {pinned}")
            };
            return Err(Short::Final(io::Error::new(
                io::ErrorKind::InvalidData,
                text,
            )));
        }
        Answer::Unusable => {
            restart(state, sink, bound.exact)?;
            return Err(Short::Retry(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "HTTP {}: the answer {status} cannot continue the download",
                    source.url
                ),
            )));
        }
    }
    progress(state.kept, state.known);
    copy_body(
        response.into_reader(),
        bound,
        state,
        sink,
        progress,
        cancelled,
    )
}

fn copy_body(
    mut reader: impl Read,
    bound: Bound,
    state: &mut State,
    sink: &mut dyn Sink,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Short> {
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if cancelled() {
            return Err(Short::Final(cancelled_error()));
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // A stalled read (the idle limit), a reset, a body cut short:
            // the next attempt continues from the bytes kept.
            Err(error) => return Err(Short::Retry(error)),
        };
        let after = state.kept.saturating_add(read as u64);
        if state.known.is_some_and(|total| after > total) {
            return Err(Short::Final(io::Error::new(
                io::ErrorKind::InvalidData,
                MORE_BYTES,
            )));
        }
        if state.known.is_none() && bound.ceiling.is_some_and(|ceiling| after > ceiling) {
            return Err(Short::Final(io::Error::other(
                "the download would leave less free space than the app data needs",
            )));
        }
        let chunk = buffer.get(..read).ok_or_else(|| {
            Short::Final(io::Error::other(
                "the reader reported more bytes than it read",
            ))
        })?;
        sink.append(chunk).map_err(Short::Final)?;
        state.kept = after;
        progress(state.kept, state.known);
    }
    match state.known {
        Some(total) if state.kept < total => Err(Short::Retry(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the server ended before its announced or signed length",
        ))),
        _ => Ok(()),
    }
}
