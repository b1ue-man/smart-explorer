//! The indicator of running screen sessions (decision C7, K5): the worker
//! starts `smart_explorer --screen-indicator` (Windows `Smart Explorer.exe`)
//! next to itself, through the same sibling lookup as the consent window.
//! The window lists the sessions it reads from the worker, ends them with
//! „Beenden“ and closes itself when none is left. Without the desktop app
//! (the worker runs as `se` alone) or without a graphical session there is
//! no indicator, and the host refuses every session with screen content
//! (`indicator_unavailable`); the test picture stays allowed. When the
//! window goes while sessions with screen content run, the worker starts it
//! again; if it fails to stay up, or went for an update, those sessions end.
//! A worker without session variables (a Linux root service, E23) has the
//! indicator in the active graphical session of `seat0` it follows: the
//! window starts there with that session's variables and account, and a
//! window started again after a session switch opens in the new session.
//! A window of a session that is no longer the active one gives way: it is
//! ended, and its end neither restarts it nor ends the running sessions.
//! Such a window reaches this worker through a channel limited to the screen
//! sessions (E46, S13, `consent_launch::spawn_window_with_channel`); the
//! channel ends with the window.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::screen::core::host_backend::SessionIndicator;
use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::session_state::EndReason;

/// The V9 start switch of the window (`StartRequest::ScreenIndicator`).
pub const SCREEN_INDICATOR_SWITCH: &str = "--screen-indicator";
/// A window that exits sooner than this after its start is not started
/// again (it cannot stay up), so a broken desktop never loops.
const SHORTEST_LIFE: Duration = Duration::from_secs(2);
/// Exit code of a window that closed for an update (V8).
const UPDATE_STOP_EXIT: i32 = crate::daemon::EXIT_UPDATE_STOP;

#[cfg(target_os = "linux")]
use crate::screen::os::session_detect::active_session::{
    process as session_window, spawn as session_target,
};

/// No other platform runs a worker outside the user's session.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
mod session_target {
    pub struct ActiveSession;
    pub fn has_own_session(_: &dyn Fn(&str) -> Option<String>) -> bool {
        true
    }
    pub fn for_worker() -> Option<ActiveSession> {
        None
    }
    pub fn apply(_: &ActiveSession, _: &mut std::process::Command) {}
    impl ActiveSession {
        pub fn generation(&self) -> &str {
            ""
        }
    }
    pub struct WindowHandle;
    impl WindowHandle {
        pub fn of(_: &std::process::Child) -> Option<Self> {
            None
        }
        pub fn terminate(&self) {}
    }
    pub fn window_moved(_: Option<&str>, _: Option<&str>) -> bool {
        false
    }
    pub fn window_closed_by_person(_: bool, _: Option<i32>) -> bool {
        false
    }
}
#[cfg(not(target_os = "linux"))]
use session_target as session_window;

/// The window this worker started last.
#[derive(Default)]
struct Window {
    running: bool,
    /// Counts the starts; only the watcher of the newest window acts.
    epoch: u64,
    /// Generation of the followed session the window runs in (E23).
    session: Option<String>,
    handle: Option<session_window::WindowHandle>,
}

struct Inner {
    program: Option<PathBuf>,
    graphical: bool,
    /// The worker has no session variables and follows the active session.
    follows: bool,
    running: Mutex<Window>,
    registry: Weak<ScreenRegistry>,
}

/// The desktop indicator of this worker.
#[derive(Clone)]
pub struct DesktopIndicator {
    inner: Arc<Inner>,
}

impl DesktopIndicator {
    /// The indicator of this installation for the sessions of `registry`.
    pub fn new(registry: &Arc<ScreenRegistry>) -> Self {
        let program = std::env::current_exe()
            .ok()
            .and_then(|current| crate::share::consent_launch::gui_program(&current, Path::is_file));
        let graphical = super::synthetic_backend::graphical_session();
        let follows = !session_target::has_own_session(&|name| std::env::var(name).ok());
        Self::build(program, graphical, follows, registry)
    }

    /// `program` is the desktop app, `graphical` whether a session exists.
    pub fn with(program: Option<PathBuf>, graphical: bool, registry: &Arc<ScreenRegistry>) -> Self {
        Self::build(program, graphical, false, registry)
    }

    fn build(
        program: Option<PathBuf>,
        graphical: bool,
        follows: bool,
        registry: &Arc<ScreenRegistry>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                program,
                graphical,
                follows,
                running: Mutex::new(Window::default()),
                registry: Arc::downgrade(registry),
            }),
        }
    }
}

impl SessionIndicator for DesktopIndicator {
    fn available(&self) -> bool {
        self.inner.program.is_some()
            && (self.inner.graphical
                || (self.inner.follows && session_target::for_worker().is_some()))
    }

    fn show(&self) -> Result<(), String> {
        if !self.available() {
            return Err("Bildschirmfreigabe braucht die Desktop-App auf dem Gerät".into());
        }
        launch(&self.inner)
    }
}

/// Starts the window unless it runs; a watcher collects its exit.
fn launch(inner: &Arc<Inner>) -> Result<(), String> {
    let mut running = inner.running.lock().unwrap_or_else(PoisonError::into_inner);
    let followed = inner.follows.then(session_target::for_worker).flatten();
    if running.running {
        let active = followed.as_ref().map(|session| session.generation());
        if !session_window::window_moved(running.session.as_deref(), active) {
            return Ok(());
        }
        // The window shows in a session that is no longer the active one.
        if let Some(handle) = running.handle.take() {
            handle.terminate();
        }
    }
    let program = inner
        .program
        .as_ref()
        .ok_or_else(|| "the desktop program is not installed next to this program".to_string())?;
    let mut command = Command::new(program);
    command
        .arg(SCREEN_INDICATOR_SWITCH)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let not_started = |error: std::io::Error| format!("indicator window not started: {error}");
    let (mut child, channel) = match (&followed, inner.follows) {
        (Some(session), true) => {
            session_target::apply(session, &mut command);
            let scope = crate::share::consent_launch::WindowScope::Indicator;
            let (child, guard) = crate::share::consent_launch::spawn_window_with_channel(
                &mut command,
                scope,
                session.generation(),
            )
            .map_err(not_started)?;
            (child, Some(guard))
        }
        (None, true) if !inner.graphical => {
            return Err("no active graphical session for the indicator window".into())
        }
        _ => (
            command.stdin(Stdio::null()).spawn().map_err(not_started)?,
            None,
        ),
    };
    running.epoch += 1;
    let epoch = running.epoch;
    running.running = true;
    running.session = followed.map(|session| session.generation().to_string());
    running.handle = inner
        .follows
        .then(|| session_window::WindowHandle::of(&child))
        .flatten();
    drop(running);
    let started = Instant::now();
    let watched = inner.clone();
    let spawned = std::thread::Builder::new()
        .name("screen-indicator".into())
        .spawn(move || {
            let code = child.wait().ok().and_then(|status| status.code());
            drop(channel);
            ended(&watched, epoch, code, started.elapsed());
        });
    if let Err(error) = spawned {
        let mut running = inner.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.epoch == epoch {
            *running = Window {
                epoch,
                ..Window::default()
            };
        }
        return Err(format!("indicator watcher not started: {error}"));
    }
    Ok(())
}

/// The window exited: start it again while sessions with screen content
/// run, or end those sessions when it cannot stay or went for an update.
fn ended(inner: &Arc<Inner>, epoch: u64, code: Option<i32>, life: Duration) {
    {
        let mut running = inner.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.epoch != epoch {
            // The window of a session that is no longer active (E23).
            return;
        }
        *running = Window {
            epoch,
            ..Window::default()
        };
    }
    let Some(registry) = inner.registry.upgrade() else {
        return;
    };
    if registry.screen_content_sessions() == 0 {
        return;
    }
    let reason = if code == Some(UPDATE_STOP_EXIT) {
        EndReason::UpdateStop
    } else if session_window::window_closed_by_person(inner.follows, code) {
        // E23: the window in the followed session closed normally; its own
        // stop request may not have reached this worker (C7).
        EndReason::HostStopped
    } else if life < SHORTEST_LIFE || launch(inner).is_err() {
        EndReason::HostStopped
    } else {
        return;
    };
    registry.end_screen_content(reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_task_indicator_needs_the_desktop_app() {
        let registry = Arc::new(ScreenRegistry::default());
        let terminal_only = DesktopIndicator::with(None, true, &registry);
        assert!(!terminal_only.available());
        let error = terminal_only.show().unwrap_err();
        assert!(error.contains("Desktop-App"), "{error}");
        let headless =
            DesktopIndicator::with(Some(PathBuf::from("smart_explorer")), false, &registry);
        assert!(!headless.available());
        let desktop =
            DesktopIndicator::with(Some(PathBuf::from("smart_explorer")), true, &registry);
        assert!(desktop.available());
    }
}
