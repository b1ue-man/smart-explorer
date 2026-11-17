//! What used to be the indicator of running screen sessions (decision C7,
//! K5). User 2026-11-17: „ich will dass es beim anzeigen und steuern eines
//! bildschirms kein popup auf b gibt“ and „das muss weg. komplett und
//! ersatzlos“: the host no longer starts any window while its screen is
//! viewed or controlled. What stays is the check the window carried: real
//! screen content needs the desktop app next to this program and a
//! graphical session (or, for a Linux root worker without session
//! variables, an active graphical session of `seat0` it follows, E23);
//! without them the host refuses sessions with screen content
//! (`indicator_unavailable`) and the test picture stays allowed. The
//! confirmation of the setting „Fragen“ is a separate dialog and unchanged.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::screen::core::host_backend::SessionIndicator;
use crate::screen::core::registry::ScreenRegistry;

#[cfg(target_os = "linux")]
use crate::screen::os::session_detect::active_session::spawn as session_target;

/// No other platform runs a worker outside the user's session.
#[cfg(not(target_os = "linux"))]
mod session_target {
    pub struct ActiveSession;
    pub fn has_own_session(_: &dyn Fn(&str) -> Option<String>) -> bool {
        true
    }
    pub fn for_worker() -> Option<ActiveSession> {
        None
    }
}

/// The screen-content check of this worker; it shows nothing.
#[derive(Clone)]
pub struct DesktopIndicator {
    program: Option<PathBuf>,
    graphical: bool,
    /// The worker has no session variables and follows the active session.
    follows: bool,
}

impl DesktopIndicator {
    /// The check of this installation (`registry`: kept for the callers).
    pub fn new(registry: &Arc<ScreenRegistry>) -> Self {
        let program = std::env::current_exe()
            .ok()
            .and_then(|current| crate::share::consent_launch::gui_program(&current, Path::is_file));
        let graphical = super::synthetic_backend::graphical_session();
        let follows = !session_target::has_own_session(&|name| std::env::var(name).ok());
        let _ = registry;
        Self {
            program,
            graphical,
            follows,
        }
    }

    /// `program` is the desktop app, `graphical` whether a session exists.
    pub fn with(program: Option<PathBuf>, graphical: bool, registry: &Arc<ScreenRegistry>) -> Self {
        let _ = registry;
        Self {
            program,
            graphical,
            follows: false,
        }
    }
}

impl SessionIndicator for DesktopIndicator {
    fn available(&self) -> bool {
        self.program.is_some()
            && (self.graphical || (self.follows && session_target::for_worker().is_some()))
    }

    /// No window (user 2026-11-17); only the check.
    fn show(&self) -> Result<(), String> {
        if !self.available() {
            return Err("Bildschirmfreigabe braucht die Desktop-App auf dem Gerät".into());
        }
        Ok(())
    }
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
