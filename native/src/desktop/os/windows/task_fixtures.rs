//! Task-test fixtures of the Windows traces (DSK-M3 signals). They work on
//! the real HKCU of the runner: snapshots of `Run`, `Software\Classes\CLSID`,
//! `Explorer\Desktop\NameSpace` and `HideDesktopIcons\NewStartPanel`, planted
//! fixed-name traces (a lost ledger) and foreign values, which they remove
//! again. `root` is unused here; the Linux fixtures keep their folders there.
use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, REG_BINARY};
use winreg::{RegKey, RegValue};

use super::integration::{CLSID_ROOT, HIDE_ICONS, NAMESPACE_ROOT};
use crate::desktop::core::ledger::{DESKTOP_NAV_CLSID, RUN_VALUE_NAME};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const FOREIGN_RUN: &str = "SmartExplorerDesktopTaskForeign";
const FOREIGN_CLSID: &str = "{5E1C3F00-0000-4000-8000-00000000D35C}";

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

fn values(key: &str, out: &mut Vec<String>) {
    if let Ok(opened) = hkcu().open_subkey(key) {
        for (name, value) in opened.enum_values().flatten() {
            out.push(format!("{key}\\{name}={value:?}"));
        }
    }
}

fn subkeys(key: &str, out: &mut Vec<String>) {
    if let Ok(opened) = hkcu().open_subkey(key) {
        for name in opened.enum_keys().flatten() {
            out.push(format!("{key}\\{name}"));
        }
    }
}

/// The four places of DSK-M3's signal `desktop_task_enable_disable_leaves_no_trace`.
pub(crate) fn trace_snapshot(_root: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    values(RUN_KEY, &mut entries);
    subkeys(CLSID_ROOT, &mut entries);
    subkeys(NAMESPACE_ROOT, &mut entries);
    values(HIDE_ICONS, &mut entries);
    entries.sort();
    entries
}

fn plant_nav_pane(clsid: &str) {
    let (key, _) = hkcu()
        .create_subkey(format!(r"{CLSID_ROOT}\{clsid}"))
        .unwrap();
    key.set_value("", &"Smart Explorer Geräte").unwrap();
    let (bag, _) = hkcu()
        .create_subkey(format!(r"{CLSID_ROOT}\{clsid}\Instance\InitPropertyBag"))
        .unwrap();
    bag.set_value("TargetFolderPath", &r"X:\").unwrap();
    let (space, _) = hkcu()
        .create_subkey(format!(r"{NAMESPACE_ROOT}\{clsid}"))
        .unwrap();
    space.set_value("", &"Smart Explorer Geräte").unwrap();
    let (icons, _) = hkcu().create_subkey(HIDE_ICONS).unwrap();
    icons.set_value(clsid, &1u32).unwrap();
}

/// Every fixed-name trace, as a lost ledger would leave them behind.
pub(crate) fn plant_fixed_traces(_root: &Path, gui: &Path) {
    super::autostart::write(gui).unwrap();
    plant_nav_pane(DESKTOP_NAV_CLSID);
}

pub(crate) fn plant_foreign(_root: &Path) {
    let (run, _) = hkcu().create_subkey(RUN_KEY).unwrap();
    run.set_value(FOREIGN_RUN, &"\"C:\\Other\\other.exe\"")
        .unwrap();
    plant_nav_pane(FOREIGN_CLSID);
}

pub(crate) fn foreign_present(_root: &Path) -> bool {
    let run = hkcu()
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(FOREIGN_RUN))
        .is_ok();
    let clsid = hkcu()
        .open_subkey(format!(
            r"{CLSID_ROOT}\{FOREIGN_CLSID}\Instance\InitPropertyBag"
        ))
        .is_ok();
    let space = hkcu()
        .open_subkey(format!(r"{NAMESPACE_ROOT}\{FOREIGN_CLSID}"))
        .is_ok();
    let icons = hkcu()
        .open_subkey(HIDE_ICONS)
        .and_then(|key| key.get_value::<u32, _>(FOREIGN_CLSID))
        .is_ok_and(|value| value == 1);
    run && clsid && space && icons
}

pub(crate) fn remove_foreign(_root: &Path) {
    if let Ok(run) = hkcu().open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_ALL_ACCESS) {
        let _ = run.delete_value(FOREIGN_RUN);
    }
    let _ = hkcu().delete_subkey_all(format!(r"{CLSID_ROOT}\{FOREIGN_CLSID}"));
    let _ = hkcu().delete_subkey_all(format!(r"{NAMESPACE_ROOT}\{FOREIGN_CLSID}"));
    if let Ok(icons) = hkcu().open_subkey_with_flags(HIDE_ICONS, winreg::enums::KEY_ALL_ACCESS) {
        let _ = icons.delete_value(FOREIGN_CLSID);
    }
}

/// Task Manager's switch-off of the login start (state 3).
pub(crate) fn switch_off_by_user(_root: &Path, _gui: &Path) {
    let (approved, _) = hkcu().create_subkey(APPROVED_KEY).unwrap();
    let value = RegValue {
        bytes: vec![3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        vtype: REG_BINARY,
    };
    approved.set_raw_value(RUN_VALUE_NAME, &value).unwrap();
}

/// The switch-off as Windows stores it; `None` when it is gone.
pub(crate) fn user_switch_off(_root: &Path) -> Option<String> {
    hkcu()
        .open_subkey(APPROVED_KEY)
        .and_then(|key| key.get_raw_value(RUN_VALUE_NAME))
        .ok()
        .map(|value| format!("{:?}", value.bytes))
}

pub(crate) fn undo_switch_off(_root: &Path) {
    if let Ok(approved) = hkcu().open_subkey_with_flags(APPROVED_KEY, winreg::enums::KEY_ALL_ACCESS)
    {
        let _ = approved.delete_value(RUN_VALUE_NAME);
    }
}

/// The sync daemon's login start `SmartExplorerSync` (F11), read only.
pub(crate) fn sync_autostart(_root: &Path) -> Option<String> {
    hkcu()
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>("SmartExplorerSync"))
        .ok()
}

/// The Explorer context-menu entry "In Smart Explorer öffnen" (F10).
pub(crate) fn context_menu_probe() -> Option<bool> {
    Some(crate::shell_register::context_menu_enabled())
}

pub(crate) fn set_context_menu(on: bool, gui: &Path) {
    crate::shell_register::set_context_menu_for(on, &gui.to_string_lossy()).unwrap();
}
