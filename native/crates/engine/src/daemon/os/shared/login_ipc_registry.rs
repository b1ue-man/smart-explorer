//! Closing admission and shared owner leases make daemon shutdown a real drain.
use crate::daemon::ipc_protocol::login::{valid_session, LoginSessionView};
use crate::login::LoginCancellation;
use std::{
    collections::BTreeMap,
    io,
    net::{Shutdown, TcpStream},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    closing: bool,
    sessions: BTreeMap<String, Arc<Entry>>,
}
static SESSIONS: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
fn table() -> &'static (Mutex<State>, Condvar) {
    SESSIONS.get_or_init(|| (Mutex::new(State::default()), Condvar::new()))
}
pub(super) struct Entry {
    pub view: Mutex<LoginSessionView>,
    pub cancel: Arc<LoginCancellation>,
    socket: TcpStream,
}
struct Lease {
    entry: Arc<Entry>,
    id: String,
}
/// Handler, transport, blocking decoder and undelivered/writing images retain
/// this lease. Only their final actual release removes the registry record.
#[derive(Clone)]
pub(super) struct Owner {
    pub entry: Arc<Entry>,
    _lease: Arc<Lease>,
}
pub(super) fn register(
    view: LoginSessionView,
    cancel: Arc<LoginCancellation>,
    socket: TcpStream,
) -> io::Result<Owner> {
    let id = view.session.clone();
    let entry = Arc::new(Entry {
        view: Mutex::new(view),
        cancel,
        socket,
    });
    let mut state = table()
        .0
        .lock()
        .map_err(|_| io::Error::other("login session registry is unavailable"))?;
    if state.closing {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "daemon login admission is closing",
        ));
    }
    // Match the existing endpoint's 64 pending application handshakes.
    if state.sessions.len() >= 64 {
        return Err(io::Error::other("login viewer session admission is full"));
    }
    if state.sessions.contains_key(&id) {
        return Err(io::Error::other("login session ID collision"));
    }
    state.sessions.insert(id.clone(), entry.clone());
    Ok(Owner {
        entry: entry.clone(),
        _lease: Arc::new(Lease { entry, id }),
    })
}
impl Entry {
    pub fn snapshot(&self) -> io::Result<LoginSessionView> {
        self.view
            .lock()
            .map(|view| view.clone())
            .map_err(|_| io::Error::other("login session state is unavailable"))
    }
    fn stop(&self) {
        self.cancel.cancel();
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.entry.stop();
        if let Ok(mut state) = table().0.lock() {
            state.sessions.remove(&self.id);
            table().1.notify_all();
        }
    }
}
pub(super) fn sessions() -> io::Result<Vec<LoginSessionView>> {
    table()
        .0
        .lock()
        .map_err(|_| io::Error::other("login session registry is unavailable"))?
        .sessions
        .values()
        .map(|entry| entry.snapshot())
        .collect()
}
pub(super) fn session(id: &str) -> io::Result<Option<LoginSessionView>> {
    if !valid_session(id) {
        return Err(io::Error::other("invalid login session ID"));
    }
    table()
        .0
        .lock()
        .map_err(|_| io::Error::other("login session registry is unavailable"))?
        .sessions
        .get(id)
        .map(|entry| entry.snapshot())
        .transpose()
}
pub(super) fn stop(id: Option<&str>) -> io::Result<usize> {
    if id.is_some_and(|id| !valid_session(id)) {
        return Err(io::Error::other("invalid login session ID"));
    }
    let entries: Vec<_> = table()
        .0
        .lock()
        .map_err(|_| io::Error::other("login session registry is unavailable"))?
        .sessions
        .iter()
        .filter(|(key, _)| id.is_none_or(|id| key.as_str() == id))
        .map(|(id, entry)| (id.clone(), entry.clone()))
        .collect();
    for (_, entry) in &entries {
        entry.stop();
    }
    let deadline = Instant::now() + Duration::from_secs(35);
    let mut state = table()
        .0
        .lock()
        .map_err(|_| io::Error::other("login session registry is unavailable"))?;
    while entries
        .iter()
        .any(|(id, _)| state.sessions.contains_key(id))
    {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "login session stop is pending; owners remain registered",
            ));
        }
        state = table()
            .1
            .wait_timeout(state, remaining)
            .map_err(|_| io::Error::other("login stop state is unavailable"))?
            .0;
    }
    Ok(entries.len())
}
/// Success proves admission closed and every registered handler/transport owner
/// released. On timeout, retain closing plus every unfinished record for retry.
pub(super) fn drain() -> io::Result<usize> {
    let entries: Vec<_> = {
        let mut state = table()
            .0
            .lock()
            .map_err(|_| io::Error::other("login session registry is unavailable"))?;
        state.closing = true;
        state.sessions.values().cloned().collect()
    };
    for entry in &entries {
        entry.stop();
    }
    let deadline = Instant::now() + Duration::from_secs(35);
    let mut state = table()
        .0
        .lock()
        .map_err(|_| io::Error::other("login session registry is unavailable"))?;
    while !state.sessions.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "login owners have not completed; keep the daemon runtime alive and drain again",
            ));
        }
        state = table()
            .1
            .wait_timeout(state, remaining)
            .map_err(|_| io::Error::other("login drain state is unavailable"))?
            .0;
    }
    Ok(entries.len())
}
