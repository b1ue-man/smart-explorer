//! E29 (M11): the switch of the GNOME desktop sharing that se manages, per
//! session. A session start switches it on and waits for its listener. The clock
//! switches it off after the grace of the last session. A withdrawal and the
//! worker's ordered end (`on_worker_stop`) switch it off at once. A crash record
//! lets the next probe switch off an orphaned sharing.
use super::super::inventory::{self, unit};
use super::net::loopback_connections;
use super::{credentials_managed, grdctl, grdctl_run, rdp_enabled, rdp_setting};
use crate::rdp::{core::SetupState, os::shared::process::capture};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard, TryLockError,
    },
    time::{Duration, Instant},
};

/// The user unit of the sharing and its daemon image, as the inventory names them.
const GNOME_UNIT: &str = "gnome-remote-desktop.service";
const GNOME_DAEMON: &str = "gnome-remote-desktop-daemon";
/// A switched-on sharing stays on this long after its last connection, or after
/// its switch-on when no connection came: the client start and a short reconnect.
pub(super) const SESSION_GRACE: Duration = Duration::from_secs(90);
/// Pause between two looks at the connections of a switched-on sharing.
const WATCH_TICK: Duration = Duration::from_secs(2);
/// Bound of the wait until the configured RDP port has a listener of the daemon.
const LISTENER_WAIT: Duration = Duration::from_secs(30);

/// The last moment a switched-on sharing had a session (E29).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SessionClock {
    last_active: Instant,
}
impl SessionClock {
    pub(super) fn started(now: Instant) -> Self {
        Self { last_active: now }
    }
    fn touch(&mut self, now: Instant) {
        self.last_active = now;
    }
    /// Connections seen at `now` are activity; none keeps the earlier moment.
    pub(super) fn observe(&mut self, connections: usize, now: Instant) {
        if connections > 0 {
            self.touch(now);
        }
    }
    /// The grace has passed since the last activity.
    pub(super) fn due(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_active) >= SESSION_GRACE
    }
}

/// The sharing this process switched on: its clock, the daemon that listens,
/// and the generation that tells its watcher whether it still belongs to it.
struct Session {
    clock: SessionClock,
    daemon_pid: u32,
    generation: u64,
}
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn session_lock() -> MutexGuard<'static, Option<Session>> {
    SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Crash record (E29): `<pid> <start time>` of the process that switched the
/// sharing on. A record whose process is gone marks an orphaned switch-on.
fn session_marker() -> std::path::PathBuf {
    crate::support_dirs::app_data_dir()
        .join("rdp")
        .join("gnome-session")
}

/// Field 22 (start time in clock ticks) of `/proc/<pid>/stat`.
fn start_time_of(pid: u32) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = text.rsplit_once(')')?.1;
    after_name.split_whitespace().nth(19).map(str::to_string)
}

/// `<pid> <start time>` of this process.
pub(super) fn own_record() -> Result<String, String> {
    let pid = std::process::id();
    let started = start_time_of(pid).ok_or_else(|| "Prozessstartzeit nicht lesbar".to_string())?;
    Ok(format!("{pid} {started}"))
}

/// Whether the process named in a record is still the one that wrote it.
pub(super) fn owner_alive(record: &str) -> bool {
    let mut fields = record.split_whitespace();
    let pid = fields.next().and_then(|field| field.parse::<u32>().ok());
    match (pid, fields.next()) {
        (Some(pid), Some(started)) => start_time_of(pid).as_deref() == Some(started),
        _ => false,
    }
}

fn write_session_marker() -> Result<(), String> {
    let record = own_record()?;
    let path = session_marker();
    if let Some(dir) = path.parent() {
        crate::support_dirs::ensure_private_dir(dir)
            .map_err(|error| format!("gnome_session_dir: {error}"))?;
    }
    crate::support_dirs::write_private_atomic(&path, format!("{record}\n").as_bytes())
        .map_err(|error| format!("gnome_session_marker: {error}"))
}

fn remove_session_marker() {
    let _ = std::fs::remove_file(session_marker());
}

/// Waits until the daemon of the sharing listens on the configured RDP port or on
/// one of the next ten ports, which GNOME may negotiate (E29).
fn wait_listener(port: u16) -> Result<u32, String> {
    let started = Instant::now();
    loop {
        let daemon = unit(GNOME_UNIT, true, GNOME_DAEMON)?;
        if daemon.active {
            let ports = inventory::listeners(daemon.pid)?;
            if ports
                .iter()
                .any(|open| (port..=port.saturating_add(10)).contains(open))
            {
                return Ok(daemon.pid);
            }
        }
        if started.elapsed() >= LISTENER_WAIT {
            return Err(
                "GNOME-Desktopfreigabe belegt den RDP-Port nach dem Einschalten nicht".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// E29 restart cleanup: a sharing that is on while no living process owns it
/// (a crash, the end of the worker, or an older version) is switched off by the
/// next probe. A living owner keeps it on. In this process a switch-on, the
/// watcher or a withdrawal holds the session lock, and a session it owns is no
/// orphan. Another process's switch-on writes its record before it enables the
/// sharing, so the status is read before the record: a sharing seen enabled by
/// a starting owner is never taken for an orphan, and only the record judged
/// orphaned is removed, never a newer owner's.
pub(super) fn release_orphan(grdctl: &Path) {
    if !credentials_managed() {
        return;
    }
    let session = match SESSION.try_lock() {
        Ok(session) => session,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if session.is_some() {
        return;
    }
    let enabled = capture(grdctl, &["status"]).is_ok_and(|status| rdp_enabled(&status));
    let record = std::fs::read_to_string(session_marker()).ok();
    if record.as_deref().is_some_and(owner_alive) {
        return;
    }
    if enabled {
        let _ = grdctl_run(grdctl, &["rdp", "disable"], None);
    }
    if std::fs::read_to_string(session_marker()).ok() == record {
        remove_session_marker();
    }
}

/// Switches the sharing on for one session (E29): the owner record first, then
/// `rdp enable`, then the wait for the listener. A failed first switch-on leaves
/// nothing switched on; a later one keeps the running session.
pub(super) fn switch_on() -> Result<(), String> {
    let grdctl = grdctl().map_err(|state| state.text())?;
    let mut session = session_lock();
    let fresh = session.is_none();
    write_session_marker()?;
    let listening = grdctl_run(&grdctl, &["rdp", "enable"], None)
        .map_err(|state| state.text())
        .and_then(|_| {
            let status = grdctl_run(&grdctl, &["status"], None).map_err(|state| state.text())?;
            let port = rdp_setting(&status, "Port")
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| "GNOME-RDP-Port nicht lesbar".to_string())?;
            wait_listener(port)
        });
    let daemon_pid = match listening {
        Ok(pid) => pid,
        Err(error) => {
            if fresh {
                let _ = grdctl_run(&grdctl, &["rdp", "disable"], None);
                remove_session_marker();
            }
            return Err(error);
        }
    };
    let now = Instant::now();
    if let Some(current) = session.as_mut() {
        current.clock.touch(now);
        current.daemon_pid = daemon_pid;
        return Ok(());
    }
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    *session = Some(Session {
        clock: SessionClock::started(now),
        daemon_pid,
        generation,
    });
    if let Err(error) = spawn_watcher(generation) {
        *session = None;
        let _ = grdctl_run(&grdctl, &["rdp", "disable"], None);
        remove_session_marker();
        return Err(error);
    }
    Ok(())
}

fn spawn_watcher(generation: u64) -> Result<(), String> {
    std::thread::Builder::new()
        .name("gnome-session-watch".into())
        .spawn(move || watch(generation))
        .map(|_| ())
        .map_err(|error| format!("GNOME-Sitzungsbeobachtung startet nicht: {error}"))
}

/// Looks at the connections of the switched-on sharing until it is switched off,
/// replaced or withdrawn; the grace after the last connection then switches it
/// off (E29). A listener list that cannot be read counts as no connection.
fn watch(generation: u64) {
    loop {
        std::thread::sleep(WATCH_TICK);
        let mut session = session_lock();
        let Some(current) = session.as_mut() else {
            return;
        };
        if current.generation != generation {
            return;
        }
        let now = Instant::now();
        let connections = inventory::listeners(current.daemon_pid)
            .map_or(0, |ports| loopback_connections(&ports));
        current.clock.observe(connections, now);
        if current.clock.due(now) && release_locked(&mut session) {
            return;
        }
    }
}

/// Switches the sharing off and forgets its session. `false` when the switch-off
/// failed, so the watcher tries again at its next tick. The caller holds the lock.
fn release_locked(session: &mut Option<Session>) -> bool {
    let Ok(grdctl) = grdctl() else {
        return false;
    };
    if grdctl_run(&grdctl, &["rdp", "disable"], None).is_err() {
        return false;
    }
    *session = None;
    remove_session_marker();
    true
}

/// Switches the sharing off at once and forgets its session: a withdrawal (no
/// device steers any more) or the worker's ordered end (`on_worker_stop`).
pub(super) fn withdraw(grdctl: &Path) -> Result<(), SetupState> {
    let mut session = session_lock();
    grdctl_run(grdctl, &["rdp", "disable"], None)?;
    *session = None;
    remove_session_marker();
    Ok(())
}
