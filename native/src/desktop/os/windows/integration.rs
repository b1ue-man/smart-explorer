//! Registrations of se Desktop on Windows (C1.4, C1.5, C5.7): the shell's
//! Run value, the navigation pane root `{DESKTOP_NAV_CLSID}` (written by the
//! shell with DSK-M5) and the jump list of the GUI image. Everything lives in
//! HKCU or in the user's profile. Removal touches only this module's fixed
//! names: the CLSID tree, the `Desktop\NameSpace` key and the one value under
//! `HideDesktopIcons\NewStartPanel`; every sibling stays.
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

use super::{autostart, is_gui_image, process};
use crate::desktop::core::ledger::{
    LedgerEntry, Registration, Trace, DESKTOP_NAV_CLSID, RUN_VALUE_NAME,
};
use crate::desktop::core::state::Platform;
use crate::desktop::os::control::{AutostartState, Integration};
use crate::desktop::os::store::unix_now;

pub(crate) const CLSID_ROOT: &str = r"Software\Classes\CLSID";
pub(crate) const NAMESPACE_ROOT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace";
pub(crate) const HIDE_ICONS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel";
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
            return clear_jump_list_here();
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
            Registration::NavPaneClsid => trace.target == DESKTOP_NAV_CLSID && nav_pane_present(),
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

/// Any of the three places of the navigation pane root (C1.4).
fn nav_pane_present() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(format!(r"{CLSID_ROOT}\{DESKTOP_NAV_CLSID}"))
        .is_ok()
        || hkcu
            .open_subkey(format!(r"{NAMESPACE_ROOT}\{DESKTOP_NAV_CLSID}"))
            .is_ok()
        || hkcu
            .open_subkey(HIDE_ICONS)
            .and_then(|key| key.get_raw_value(DESKTOP_NAV_CLSID))
            .is_ok()
}

fn not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
}

/// Deletes the CLSID tree, the `NameSpace` key and the `HideDesktopIcons`
/// value of this module's CLSID, then tells the shell (C1.4).
fn remove_nav_pane(clsid: &str) -> Result<(), String> {
    if clsid != DESKTOP_NAV_CLSID {
        return Err(format!("fremde CLSID {clsid:?} wird nicht gelöscht"));
    }
    if !nav_pane_present() {
        return Ok(());
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for key in [
        format!(r"{CLSID_ROOT}\{clsid}"),
        format!(r"{NAMESPACE_ROOT}\{clsid}"),
    ] {
        match hkcu.delete_subkey_all(&key) {
            Ok(()) => {}
            Err(error) if not_found(&error) => {}
            Err(error) => return Err(format!("{key}: {error}")),
        }
    }
    match hkcu.open_subkey_with_flags(HIDE_ICONS, KEY_READ | KEY_SET_VALUE) {
        Ok(icons) => match icons.delete_value(clsid) {
            Ok(()) => {}
            Err(error) if not_found(&error) => {}
            Err(error) => return Err(format!("{HIDE_ICONS}: {error}")),
        },
        Err(error) if not_found(&error) => {}
        Err(error) => return Err(format!("{HIDE_ICONS}: {error}")),
    }
    notify_assoc_changed();
    Ok(())
}

fn notify_assoc_changed() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

/// `DeleteList(NULL)` for the jump list of this process's image
/// (`--desktop-jumplist-clear`, `--unregister`).
pub(crate) fn clear_jump_list_here() -> Result<(), String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{DestinationList, ICustomDestinationList};
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = CoCreateInstance::<_, ICustomDestinationList>(
            &DestinationList,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .and_then(|list| list.DeleteList(windows::core::PCWSTR::null()));
        if initialized {
            CoUninitialize();
        }
        result.map_err(|error| format!("Sprungliste löschen: {error}"))
    }
}
