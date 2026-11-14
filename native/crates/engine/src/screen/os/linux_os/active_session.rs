//! The active graphical session of `seat0` for a worker without session
//! variables (decision E23, security S3): a root service, or a worker of the
//! session's own account started without `DISPLAY`/`WAYLAND_DISPLAY`. logind
//! names the session (`Seat.ActiveSession`, `Session.Type`, `Class`,
//! `Display`, `User`, `Leader`); its environment comes only from processes
//! whose four UIDs are the session's (`active_session_env`); the screen host
//! and the indicator window then run in that session as its account
//! (`active_session_spawn`). Whatever is not unique and verified gives no
//! session, so the host answers `no_display` and never guesses an
//! environment. A worker with its own session variables never comes here.
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::SessionKind;

#[path = "active_session_backend.rs"]
pub mod backend;
#[cfg(test)]
#[path = "active_session_daemon_task_tests.rs"]
mod daemon_tests;
#[path = "active_session_env.rs"]
pub mod env;
#[path = "active_session_process.rs"]
pub mod process;
#[path = "active_session_spawn.rs"]
pub mod spawn;
#[cfg(test)]
#[path = "active_session_task_tests.rs"]
mod tests;
#[path = "active_session_x11.rs"]
pub mod x11;

pub use env::{ProcessTable, SystemProcesses};
pub use spawn::{Account, Accounts, SystemAccounts};

/// How long one look at logind and `/proc` stays valid; a session switch
/// is seen within this time.
const FRESH_FOR: Duration = Duration::from_secs(1);

/// The logind facts of one session (D-Bus API of systemd 262).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogindSession {
    pub id: String,
    /// `user`, `greeter` or `lock-screen`.
    pub class: String,
    /// `x11`, `wayland`, `mir`, `tty`, `web` or `unspecified`.
    pub kind: String,
    pub active: bool,
    pub remote: bool,
    /// `online`, `active` or `closing`.
    pub state: String,
    pub uid: u32,
    pub leader: u32,
    /// The X11 display of a graphical login, empty otherwise.
    pub display: String,
    pub timestamp: u64,
    /// `User.RuntimePath` of the session's account (`XDG_RUNTIME_DIR`).
    pub runtime_path: String,
}

impl LogindSession {
    /// Changes with every new session, account, leader or start time; a
    /// running host belongs to exactly one value.
    pub fn generation(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.id, self.timestamp, self.uid, self.leader
        )
    }
}

/// Source of the active session of `seat0`.
pub trait Logind {
    /// `None` when `seat0` has no active session.
    fn active_session(&self) -> io::Result<Option<LogindSession>>;
}

/// The system's logind over the system bus.
pub struct SystemLogind;

impl Logind for SystemLogind {
    /// Outside any Tokio runtime: the host backend asks from async tasks, and
    /// zbus's blocking API panics on a thread that drives Tokio (`off_runtime`).
    fn active_session(&self) -> io::Result<Option<LogindSession>> {
        crate::screen::os::app_units::off_runtime(system_active_session)
    }
}

/// Why no session is followed; every case is `no_display` for the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// logind does not answer.
    LogindUnavailable,
    /// `seat0` has no active session.
    NoActiveSession,
    /// The session is not in the foreground, closing or remote.
    NotLocalForeground,
    /// Neither X11 nor Wayland, or a class without a screen.
    NotGraphical,
    /// A session of root itself (decision: never run the host as root in a
    /// desktop session).
    RootSession,
    /// The worker is neither root nor the session's account.
    ForeignAccount,
    /// The account of the session is not in the account database.
    UnknownAccount,
    /// The environment is missing, ambiguous or not owned by the account.
    Environment(&'static str),
    /// A screen host process started by a worker: only the worker decides
    /// its session (`process::HOST_ENV`).
    StartedHost,
}

/// The followed session: who it is and what its processes run with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSession {
    pub id: String,
    pub kind: SessionKind,
    pub account: Account,
    /// Session variables for processes started in this session.
    pub env: Vec<(String, String)>,
    generation: String,
}

impl ActiveSession {
    /// The value `still_current` compares.
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// A followed session of the task tests (simulated logind answer).
    #[cfg(test)]
    pub fn simulated(kind: SessionKind, account: Account, env: Vec<(String, String)>) -> Self {
        Self {
            id: "7".into(),
            kind,
            account,
            env,
            generation: "7:1:1000:42".into(),
        }
    }
}

/// The active session of `seat0` as a worker with effective UID
/// `own_euid` may follow it, or why not.
pub fn resolve(
    logind: &dyn Logind,
    processes: &dyn ProcessTable,
    accounts: &dyn Accounts,
    own_euid: u32,
) -> Result<ActiveSession, Unresolved> {
    let session = logind
        .active_session()
        .map_err(|_| Unresolved::LogindUnavailable)?
        .filter(|session| !session.id.is_empty())
        .ok_or(Unresolved::NoActiveSession)?;
    if !session.active || session.remote || session.state == "closing" {
        return Err(Unresolved::NotLocalForeground);
    }
    if !matches!(session.class.as_str(), "user" | "greeter" | "lock-screen") {
        return Err(Unresolved::NotGraphical);
    }
    let kind = match session.kind.as_str() {
        "x11" => SessionKind::X11,
        "wayland" => SessionKind::Wayland,
        _ => return Err(Unresolved::NotGraphical),
    };
    if session.uid == 0 {
        return Err(Unresolved::RootSession);
    }
    if own_euid != 0 && own_euid != session.uid {
        return Err(Unresolved::ForeignAccount);
    }
    let account = accounts
        .account(session.uid)
        .ok_or(Unresolved::UnknownAccount)?;
    let env = env::collect(processes, &session, kind).map_err(Unresolved::Environment)?;
    Ok(ActiveSession {
        id: session.id.clone(),
        kind,
        account,
        env,
        generation: session.generation(),
    })
}

struct Cached<T> {
    at: Instant,
    value: T,
}

static CURRENT: Mutex<Option<Cached<Result<ActiveSession, Unresolved>>>> = Mutex::new(None);
static GENERATION: Mutex<Option<Cached<Option<String>>>> = Mutex::new(None);

/// The session this worker follows now (logind and `/proc` read at most
/// once per `FRESH_FOR`).
pub fn current() -> Result<ActiveSession, Unresolved> {
    if process::is_started_host(std::env::var(process::HOST_ENV).ok().as_deref()) {
        return Err(Unresolved::StartedHost);
    }
    let mut slot = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(cached) = slot
        .as_ref()
        .filter(|cached| cached.at.elapsed() < FRESH_FOR)
    {
        return cached.value.clone();
    }
    // SAFETY: geteuid takes no arguments and cannot fail.
    let own_euid = unsafe { libc::geteuid() };
    let value = resolve(&SystemLogind, &SystemProcesses, &SystemAccounts, own_euid);
    *slot = Some(Cached {
        at: Instant::now(),
        value: value.clone(),
    });
    value
}

/// The generation of the active session of `seat0` (logind only, at most
/// once per `FRESH_FOR`); `None` without one or without logind.
pub fn current_generation() -> Option<String> {
    let mut slot = GENERATION.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(cached) = slot
        .as_ref()
        .filter(|cached| cached.at.elapsed() < FRESH_FOR)
    {
        return cached.value.clone();
    }
    let value = SystemLogind
        .active_session()
        .ok()
        .flatten()
        .filter(|session| !session.id.is_empty())
        .map(|session| session.generation());
    *slot = Some(Cached {
        at: Instant::now(),
        value: value.clone(),
    });
    value
}

/// Whether a host started for `generation` still belongs to the active
/// session; an unknown state counts as switched (fail closed, S3).
pub fn is_current(generation: &str, active: Option<&str>) -> bool {
    active == Some(generation)
}

fn system_active_session() -> io::Result<Option<LogindSession>> {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;
    const SERVICE: &str = "org.freedesktop.login1";
    fn property<T>(proxy: &Proxy<'_>, key: &str) -> io::Result<T>
    where
        T: serde::de::DeserializeOwned + TryFrom<zbus::zvariant::OwnedValue>,
        T::Error: Into<zbus::Error>,
    {
        proxy.get_property(key).map_err(io::Error::other)
    }
    let connection = Connection::system().map_err(io::Error::other)?;
    let seat = Proxy::new(
        &connection,
        SERVICE,
        "/org/freedesktop/login1/seat/seat0",
        "org.freedesktop.login1.Seat",
    )
    .map_err(io::Error::other)?;
    let (id, path): (String, OwnedObjectPath) = property(&seat, "ActiveSession")?;
    if id.is_empty() {
        return Ok(None);
    }
    let session = Proxy::new(
        &connection,
        SERVICE,
        path.as_str(),
        "org.freedesktop.login1.Session",
    )
    .map_err(io::Error::other)?;
    let (uid, user_path): (u32, OwnedObjectPath) = property(&session, "User")?;
    let user = Proxy::new(
        &connection,
        SERVICE,
        user_path.as_str(),
        "org.freedesktop.login1.User",
    )
    .map_err(io::Error::other)?;
    Ok(Some(LogindSession {
        id,
        class: property(&session, "Class")?,
        kind: property(&session, "Type")?,
        active: property(&session, "Active")?,
        remote: property(&session, "Remote")?,
        state: property(&session, "State")?,
        uid,
        leader: property(&session, "Leader")?,
        display: property(&session, "Display")?,
        timestamp: property(&session, "TimestampMonotonic")?,
        runtime_path: property(&user, "RuntimePath")?,
    }))
}
