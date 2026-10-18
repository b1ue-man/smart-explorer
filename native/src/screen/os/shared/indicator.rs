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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::screen::core::host_backend::SessionIndicator;
use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::session_state::EndReason;

/// The V9 start switch of the window (`StartRequest::ScreenIndicator`).
pub(crate) const SCREEN_INDICATOR_SWITCH: &str = "--screen-indicator";
/// A window that exits sooner than this after its start is not started
/// again (it cannot stay up), so a broken desktop never loops.
const SHORTEST_LIFE: Duration = Duration::from_secs(2);
/// Exit code of a window that closed for an update (V8).
const UPDATE_STOP_EXIT: i32 = crate::daemon::EXIT_UPDATE_STOP;

struct Inner {
    program: Option<PathBuf>,
    graphical: bool,
    running: Mutex<bool>,
    registry: Weak<ScreenRegistry>,
}

/// The desktop indicator of this worker.
#[derive(Clone)]
pub(crate) struct DesktopIndicator {
    inner: Arc<Inner>,
}

impl DesktopIndicator {
    /// The indicator of this installation for the sessions of `registry`.
    pub(crate) fn new(registry: &Arc<ScreenRegistry>) -> Self {
        let program = std::env::current_exe()
            .ok()
            .and_then(|current| crate::share::consent_launch::gui_program(&current, Path::is_file));
        let graphical = super::synthetic_backend::graphical_session();
        Self::with(program, graphical, registry)
    }

    /// `program` is the desktop app, `graphical` whether a session exists.
    pub(crate) fn with(
        program: Option<PathBuf>,
        graphical: bool,
        registry: &Arc<ScreenRegistry>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                program,
                graphical,
                running: Mutex::new(false),
                registry: Arc::downgrade(registry),
            }),
        }
    }
}

impl SessionIndicator for DesktopIndicator {
    fn available(&self) -> bool {
        self.inner.graphical && self.inner.program.is_some()
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
    if *running {
        return Ok(());
    }
    let program = inner
        .program
        .as_ref()
        .ok_or_else(|| "the desktop program is not installed next to this program".to_string())?;
    let mut child = Command::new(program)
        .arg(SCREEN_INDICATOR_SWITCH)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("indicator window not started: {error}"))?;
    *running = true;
    drop(running);
    let started = Instant::now();
    let watched = inner.clone();
    let spawned = std::thread::Builder::new()
        .name("screen-indicator".into())
        .spawn(move || {
            let code = child.wait().ok().and_then(|status| status.code());
            ended(&watched, code, started.elapsed());
        });
    if let Err(error) = spawned {
        *inner.running.lock().unwrap_or_else(PoisonError::into_inner) = false;
        return Err(format!("indicator watcher not started: {error}"));
    }
    Ok(())
}

/// The window exited: start it again while sessions with screen content
/// run, or end those sessions when it cannot stay or went for an update.
fn ended(inner: &Arc<Inner>, code: Option<i32>, life: Duration) {
    *inner.running.lock().unwrap_or_else(PoisonError::into_inner) = false;
    let Some(registry) = inner.registry.upgrade() else {
        return;
    };
    if registry.screen_content_sessions() == 0 {
        return;
    }
    let reason = if code == Some(UPDATE_STOP_EXIT) {
        EndReason::UpdateStop
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
