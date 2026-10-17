//! Windows adapter of se Desktop (DSK-M3): login start, single instance,
//! processes, registrations, monitors and the facts the core decides on.
//! The lowest tool window, wallpaper, desktop folder, navigation pane and
//! jump list writing follow with DSK-M5 in their own files.
use std::path::Path;

use sha2::{Digest, Sha256};

#[path = "autostart.rs"]
mod autostart;
#[path = "instance.rs"]
pub(crate) mod instance;
#[path = "integration.rs"]
mod integration;
#[path = "process.rs"]
pub(crate) mod process;
#[cfg(test)]
#[path = "task_fixtures.rs"]
pub(crate) mod task_fixtures;

pub(crate) use integration::{clear_jump_list_here, WindowsIntegration as PlatformIntegration};

use super::Monitor;
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::items::SystemItem;
use crate::desktop::core::mechanism::{DisplayServer, SessionFacts};
use crate::desktop::core::state::Platform;

pub(crate) const PLATFORM: Platform = Platform::Windows;
/// A halted shell stays while `notify_desktop` shows its balloon (30 s).
pub(crate) const NOTICE_LINGER: std::time::Duration = std::time::Duration::from_secs(31);

/// Names of the GUI program (`updater` installs `Smart Explorer.exe`;
/// development builds run `smart_explorer.exe`).
const GUI_IMAGES: [&str; 2] = ["Smart Explorer.exe", "smart_explorer.exe"];

pub(crate) fn is_gui_image(exe: &Path) -> bool {
    exe.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| GUI_IMAGES.iter().any(|gui| gui.eq_ignore_ascii_case(name)))
}

/// Windows always offers the lowest tool window (C1.2).
pub(crate) fn session_facts() -> SessionFacts {
    SessionFacts {
        display: DisplayServer::Windows,
        ..SessionFacts::default()
    }
}

/// Start values of "Papierkorb" and "Dieser PC" from Explorer's
/// `HideDesktopIcons\NewStartPanel` values, read once at activation (Ä5).
pub(crate) fn initial_system_items() -> SystemItems {
    use winreg::enums::HKEY_CURRENT_USER;
    let key = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
    );
    let read = |item: SystemItem| {
        key.as_ref()
            .ok()
            .and_then(|key| key.get_value::<u32, _>(item.hide_icons_value()).ok())
    };
    SystemItems::initial(read(SystemItem::RecycleBin), read(SystemItem::ThisPc))
}

/// Every monitor of the desktop (`EnumDisplayMonitors`). The key is the
/// first 16 hex characters of SHA-256 over the GDI device name; DSK-M5
/// derives it from the `IDesktopWallpaper` device path instead (C1.2).
pub(crate) fn monitors() -> Vec<Monitor> {
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    const PRIMARY: u32 = 1;

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<Monitor>);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
        .as_bool()
        {
            let end = info
                .szDevice
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(info.szDevice.len());
            let device = String::from_utf16_lossy(&info.szDevice[..end]);
            let rect = info.monitorInfo.rcMonitor;
            list.push(Monitor {
                key: monitor_key(&device),
                primary: info.monitorInfo.dwFlags & PRIMARY != 0,
                rect: Some([rect.left, rect.top, rect.right, rect.bottom]),
            });
        }
        BOOL(1)
    }

    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC(std::ptr::null_mut()),
            None,
            Some(collect),
            LPARAM(&mut list as *mut Vec<Monitor> as isize),
        );
    }
    list
}

fn monitor_key(device: &str) -> String {
    let digest = Sha256::digest(device.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
