//! Registrations of se Desktop on Windows (C1.4, C1.5, C5.7): the shell's
//! Run value, the navigation pane root `{DESKTOP_NAV_CLSID}` with the parent
//! keys it created (written by the shell, `shell_parts`) and the jump list
//! of the GUI image. Everything lives in HKCU or in the user's profile.
//! Removal touches only this module's fixed names: the CLSID tree, the
//! `Desktop\NameSpace` key, the one value under `HideDesktopIcons\
//! NewStartPanel` and parent keys se created while they hold nothing else;
//! every sibling stays.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{autostart, is_gui_image, jump_list, nav_pane, process};
use crate::desktop::core::ledger::{
    LedgerEntry, Registration, Trace, DESKTOP_NAV_CLSID, RUN_VALUE_NAME,
};
use crate::desktop::core::state::Platform;
use crate::desktop::os::control::{AutostartState, Integration};
use crate::desktop::os::store::unix_now;

/// `--desktop-jumplist-clear` is one COM call; this bounds a hanging start.
const JUMP_LIST_WAIT: Duration = Duration::from_secs(15);

pub(crate) struct WindowsIntegration {
    /// The GUI program: the jump list can only be cleared from its image.
    gui: Option<PathBuf>,
    /// This process is of the GUI image and clears the list itself.
    jump_list_here: bool,
}

impl WindowsIntegration {
    pub(crate) fn new(gui: Option<PathBuf>) -> WindowsIntegration {
        let jump_list_here = std::env::current_exe().is_ok_and(|exe| is_gui_image(&exe));
        WindowsIntegration {
            gui,
            jump_list_here,
        }
    }

    /// Task tests clear the test process's own (empty) jump list.
    #[cfg(test)]
    pub(crate) fn for_task(_root: &Path) -> WindowsIntegration {
        WindowsIntegration {
            gui: None,
            jump_list_here: true,
        }
    }

    /// C1.5: only a process of the GUI image reaches the list without an
    /// explicit AppUserModelID; another image starts the GUI program with
    /// `--desktop-jumplist-clear` and waits for it.
    fn clear_jump_list(&self) -> Result<(), String> {
        if self.jump_list_here {
            return jump_list::clear_here();
        }
        let gui = self
            .gui
            .as_deref()
            .ok_or("ohne Smart-Explorer-App lässt sich die Sprungliste nicht löschen")?;
        let mut child = process::spawn_detached(gui, &["--desktop-jumplist-clear"])
            .map_err(|error| format!("Sprungliste löschen: {error}"))?;
        let deadline = Instant::now() + JUMP_LIST_WAIT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("Sprungliste löschen: {status}")),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Sprungliste löschen: keine Antwort".into());
                }
                Err(error) => return Err(format!("Sprungliste löschen: {error}")),
            }
        }
    }
}

impl Integration for WindowsIntegration {
    fn platform(&self) -> Platform {
        Platform::Windows
    }

    fn autostart_registration(&self) -> Registration {
        Registration::RunValue
    }

    fn apply(&mut self, registration: Registration, gui: &Path) -> Result<LedgerEntry, String> {
        match registration {
            Registration::RunValue => {
                // A switch-off in StartupApproved stays: se never writes it.
                autostart::write(gui).map_err(|error| error.to_string())?;
                Ok(LedgerEntry {
                    kind: registration,
                    target: RUN_VALUE_NAME.into(),
                    applied_at: unix_now(),
                })
            }
            other => Err(format!(
                "{} legt nur der se-Desktop-Wächter an",
                other.key()
            )),
        }
    }

    fn unapply(&mut self, trace: &Trace) -> Result<(), String> {
        match trace.kind {
            Registration::RunValue => {
                autostart::remove(&trace.target).map_err(|error| error.to_string())
            }
            Registration::NavPaneClsid => remove_nav_pane(&trace.target),
            Registration::JumpList => self.clear_jump_list(),
            // Linux registrations do not exist here.
            _ => Ok(()),
        }
    }

    fn is_applied(&self, trace: &Trace) -> bool {
        match trace.kind {
            Registration::RunValue => trace.target == RUN_VALUE_NAME && autostart::present(),
            Registration::NavPaneClsid if trace.target == DESKTOP_NAV_CLSID => nav_pane::present(),
            Registration::NavPaneClsid => nav_pane::parent_left(&trace.target),
            // Not readable (Annahme A6): removed once `DeleteList` succeeded.
            _ => false,
        }
    }

    fn autostart(&self) -> AutostartState {
        if !autostart::present() {
            AutostartState::Off
        } else if autostart::disabled_by_system() {
            AutostartState::DisabledBySystem
        } else {
            AutostartState::On
        }
    }

    fn runtime_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// The root of this module's CLSID, or a parent key the root created;
/// a foreign CLSID or key is refused.
fn remove_nav_pane(target: &str) -> Result<(), String> {
    if target == DESKTOP_NAV_CLSID {
        nav_pane::remove()
    } else {
        nav_pane::remove_parent(target)
    }
}
