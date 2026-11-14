//! Queries of a worker that follows the active session (E23, E47, S3): the
//! window list with its icons, one icon and the encoder offer come from a
//! short host process in the session's account, never from a root process
//! against that account's bus, compositor or codec store. The process is the
//! host process of this installation (`--screen-host-worker`), started like
//! a session's host (`spawn::apply`, `process::mark_host`, parent check and
//! parent-death signal), with `SE_SCREEN_SESSION_QUERY` set: it reads one
//! query (JSON) from its standard input, answers once (JSON) on its standard
//! output and ends. A worker with its own session variables never asks; its
//! paths stay as before. The answer is bounded and checked here: an icon
//! must match the reference of the authorized list, a missing or failed
//! answer is an unknown list and the lossless tiles.
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "linux"))]
use super::session_target::ActiveSession;
use super::{session_mark, session_target, PARENT_ENV, SCREEN_HOST_WORKER_SWITCH};
use crate::screen::core::app_icon::{AuthorizedIconEntry, IconRef, IconReply};
use crate::screen::core::host_backend::ScreenHostBackend;
use crate::screen::core::negotiate::{CodecKind, CodecOffer};
use crate::screen::core::window_list::{AppEntry, WindowEntry, WindowList};
use crate::screen::core::wire::{from_json, to_json, MAX_WINDOWS_BYTES};
#[cfg(target_os = "linux")]
use crate::screen::os::session_detect::active_session::ActiveSession;

/// Mark of a host process that answers one session query.
pub const QUERY_ENV: &str = "SE_SCREEN_SESSION_QUERY";
/// Time a query process gets for its answer.
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
/// Time a query process gets to exit after its answer.
const EXIT_WAIT: Duration = Duration::from_secs(1);
/// Largest query (an icon query carries one list entry).
const MAX_QUERY_BYTES: usize = MAX_WINDOWS_BYTES;
/// Largest answer (a window list, or one icon with its PNG).
const MAX_ANSWER_BYTES: usize = 4 * MAX_WINDOWS_BYTES;
/// Most encoder offers of this host, the tiles included: a hello lists at
/// most 32 encoders (`ScreenHelloOk::validate`), and a viewer refuses more.
const MAX_OFFERS: usize = 32;
/// How long an encoder offer of the account stays valid (the person may
/// switch OpenH264 on or off).
const OFFERS_FRESH: Duration = Duration::from_secs(5);

/// One query to the session's account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionQuery {
    /// `windows()` of the platform backend, icons attached.
    Windows,
    /// The encoder offer of the account's codec inventory.
    Encoders,
    /// The icon of one entry of an authorized list.
    Icon {
        #[serde(default)]
        window: Option<WindowEntry>,
        #[serde(default)]
        app: Option<AppEntry>,
    },
}

/// The answer of the session's account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAnswer {
    Windows(WindowList),
    Encoders(Vec<CodecOffer>),
    Icon(Option<IconReply>),
    Failed(String),
}

/// The command of a query process for `session`.
pub fn command_for(program: &Path, session: &ActiveSession) -> Command {
    let mut command = Command::new(program);
    session_target::apply(session, &mut command);
    session_mark::mark_host(Some(session), &mut command);
    command
        .arg(SCREEN_HOST_WORKER_SWITCH)
        .env(PARENT_ENV, std::process::id().to_string())
        .env(QUERY_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    super::super::worker_lifetime::prepare(&mut command);
    command
}

/// Asks `query` in a query process of `program` in `session`.
pub fn ask_in(
    program: &Path,
    session: &ActiveSession,
    query: &SessionQuery,
) -> io::Result<SessionAnswer> {
    let request = to_json(query)?;
    let mut child = super::spawn(command_for(program, session))?;
    let stop = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    if let Err(error) = super::super::worker_lifetime::bind(&child) {
        stop(&mut child);
        return Err(error);
    }
    let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        stop(&mut child);
        return Err(io::Error::other("screen session query has no pipes"));
    };
    let (sent, answer) = mpsc::channel();
    let talk = std::thread::Builder::new()
        .name("screen-session-query".into())
        .spawn(move || {
            let _ = sent.send(talk(input, output, &request));
        });
    if let Err(error) = talk {
        stop(&mut child);
        return Err(error);
    }
    let read = answer.recv_timeout(QUERY_TIMEOUT);
    if super::wait_for(&mut child, EXIT_WAIT).is_none() {
        stop(&mut child);
    }
    let bytes = match read {
        Ok(read) => read?,
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the screen session query did not answer",
            ))
        }
    };
    if bytes.len() > MAX_ANSWER_BYTES {
        return Err(io::Error::other("screen session answer exceeds its limit"));
    }
    from_json(&bytes)
}

/// Writes the query, closes the input and reads the bounded answer.
fn talk(mut input: impl Write, output: impl Read, request: &[u8]) -> io::Result<Vec<u8>> {
    input.write_all(request)?;
    drop(input);
    let mut bytes = Vec::new();
    output
        .take(MAX_ANSWER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Asks `query` in the followed session with the desktop program of this
/// installation; an error without a followed session or program.
pub fn ask(query: &SessionQuery) -> io::Result<SessionAnswer> {
    let session =
        session_target::for_worker().ok_or_else(|| io::Error::other("no followed session"))?;
    let program = super::desktop_program()
        .0
        .ok_or_else(|| io::Error::other("the desktop program is not installed"))?;
    ask_in(&program, &session, query)
}

/// The window list of an answer.
pub fn windows_of(answer: io::Result<SessionAnswer>) -> io::Result<WindowList> {
    match answer? {
        SessionAnswer::Windows(list) => Ok(list),
        SessionAnswer::Failed(message) => Err(io::Error::other(message)),
        _ => Err(io::Error::other("unexpected screen session answer")),
    }
}

/// The encoder offer of an answer: at most `MAX_OFFERS` with the tiles,
/// which are always there (every host process has them); only the tiles
/// without an answer.
pub fn offers_of(answer: io::Result<SessionAnswer>) -> Vec<CodecOffer> {
    let mut offers = match answer {
        Ok(SessionAnswer::Encoders(offers)) => offers,
        _ => Vec::new(),
    };
    offers.truncate(MAX_OFFERS);
    if !offers.iter().any(|offer| offer.codec == CodecKind::Tiles) {
        offers.truncate(MAX_OFFERS - 1);
        offers.push(CodecOffer::tiles());
    }
    offers
}

/// The icon of an answer, only when it is the one `expected` names.
pub fn icon_of(
    answer: io::Result<SessionAnswer>,
    expected: &IconRef,
) -> io::Result<Option<IconReply>> {
    match answer? {
        SessionAnswer::Icon(reply) => Ok(reply.filter(|reply| &reply.icon == expected)),
        SessionAnswer::Failed(message) => Err(io::Error::other(message)),
        _ => Err(io::Error::other("unexpected screen session answer")),
    }
}

/// The query of the icon of `entry` and the reference it must match.
pub fn icon_query(entry: AuthorizedIconEntry<'_>) -> (SessionQuery, Option<IconRef>) {
    match entry {
        AuthorizedIconEntry::Window(window) => (
            SessionQuery::Icon {
                window: Some(window.clone()),
                app: None,
            },
            window.icon.clone(),
        ),
        AuthorizedIconEntry::App(app) => (
            SessionQuery::Icon {
                window: None,
                app: Some(app.clone()),
            },
            app.icon.clone(),
        ),
    }
}

/// Encoder offer, followed session generation and when it was asked.
type CachedOffers = (String, Instant, Vec<CodecOffer>);
static OFFERS: Mutex<Option<CachedOffers>> = Mutex::new(None);

/// The encoder offer of the account of the followed session for host
/// processes of `program`; `None` for a worker that follows no session.
pub fn followed_offers(program: &Path) -> Option<Vec<CodecOffer>> {
    let session = session_target::for_worker()?;
    let generation = session.generation().to_string();
    let mut slot = OFFERS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, _, offers)) = slot
        .as_ref()
        .filter(|(known, at, _)| *known == generation && at.elapsed() < OFFERS_FRESH)
    {
        return Some(offers.clone());
    }
    let offers = offers_of(ask_in(program, &session, &SessionQuery::Encoders));
    *slot = Some((generation, Instant::now(), offers.clone()));
    Some(offers)
}

/// Whether this host process answers one session query.
pub fn requested() -> bool {
    std::env::var_os(QUERY_ENV).is_some()
}

/// The answer of this process (the session's account) to `query`.
pub fn answer(
    query: SessionQuery,
    backend: &dyn ScreenHostBackend,
    offers: impl FnOnce() -> Vec<CodecOffer>,
) -> SessionAnswer {
    let failed = |error: io::Error| SessionAnswer::Failed(error.to_string());
    match query {
        SessionQuery::Windows => backend
            .windows()
            .map_or_else(failed, SessionAnswer::Windows),
        SessionQuery::Encoders => SessionAnswer::Encoders(offers()),
        SessionQuery::Icon {
            window: Some(window),
            ..
        } => backend
            .icon(AuthorizedIconEntry::Window(&window))
            .map_or_else(failed, SessionAnswer::Icon),
        SessionQuery::Icon { app: Some(app), .. } => backend
            .icon(AuthorizedIconEntry::App(&app))
            .map_or_else(failed, SessionAnswer::Icon),
        SessionQuery::Icon { .. } => SessionAnswer::Icon(None),
    }
}

/// The query process: one query from `input`, one answer to `output`; the
/// result is its exit code.
pub fn serve(
    input: impl Read,
    mut output: impl Write,
    backend: &dyn ScreenHostBackend,
    offers: impl FnOnce() -> Vec<CodecOffer>,
) -> i32 {
    let mut bytes = Vec::new();
    let read = input
        .take(MAX_QUERY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .and_then(|_| {
            if bytes.len() > MAX_QUERY_BYTES {
                return Err(io::Error::other("screen session query exceeds its limit"));
            }
            from_json::<SessionQuery>(&bytes)
        });
    let reply = match read {
        Ok(query) => answer(query, backend, offers),
        Err(error) => SessionAnswer::Failed(error.to_string()),
    };
    let written = to_json(&reply).and_then(|bytes| {
        output.write_all(&bytes)?;
        output.flush()
    });
    match written {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("screen session query: {error}");
            1
        }
    }
}
