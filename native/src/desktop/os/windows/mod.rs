//! Windows adapter of se Desktop: login start, single instance, processes
//! and registrations (DSK-M3), and with DSK-M5 the lowest tool window of
//! each monitor, the monitors with their keys, the system wallpaper, the
//! desktop folders, the start values of "Papierkorb" and "Dieser PC", the
//! Explorer navigation pane, the jump list and the Explorer actions of the
//! surface. Shared code reaches all of it through the names exported here;
//! the Linux adapter exports the same names.
use std::path::Path;

use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod adapter_tests;
#[path = "autostart.rs"]
mod autostart;
#[path = "desktop_folder.rs"]
pub(crate) mod desktop_folder;
#[path = "instance.rs"]
pub(crate) mod instance;
#[path = "integration.rs"]
mod integration;
#[path = "jump_list.rs"]
pub(crate) mod jump_list;
#[path = "monitors.rs"]
pub(crate) mod monitors;
#[path = "nav_pane.rs"]
pub(crate) mod nav_pane;
#[path = "process.rs"]
pub(crate) mod process;
#[cfg(test)]
#[path = "registry_task_tests.rs"]
mod registry_task_tests;
#[path = "shell_actions.rs"]
mod shell_actions;
#[path = "shell_parts.rs"]
mod shell_parts;
#[path = "surface_window.rs"]
pub(crate) mod surface_window;
#[path = "system_items.rs"]
pub(crate) mod system_items;
#[cfg(test)]
#[path = "task_fixtures.rs"]
pub(crate) mod task_fixtures;
#[path = "wallpaper.rs"]
pub(crate) mod wallpaper;
#[cfg(test)]
#[path = "window_task_env.rs"]
mod window_task_env;
#[cfg(test)]
#[path = "window_task_tests.rs"]
mod window_task_tests;

pub(crate) use desktop_folder::{desktop_folders, hidden_entry};
pub(crate) use integration::WindowsIntegration as PlatformIntegration;
pub(crate) use jump_list::clear_here as clear_jump_list_here;
pub(crate) use monitors::{monitor_names, monitors, work_area};
pub(crate) use shell_actions::{pointer_outside, release_pointer, shell_action};
pub(crate) use shell_parts::ShellParts;
pub(crate) use surface_window::attach_surface;
pub(crate) use system_items::initial_system_items;
pub(crate) use wallpaper::system_wallpaper;

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

/// COM on the calling thread for one call: a single-threaded apartment
/// when the thread has none; an apartment the thread already has (winit's
/// OLE on a surface's UI thread) stays as it is.
pub(crate) struct ComApartment(bool);

impl ComApartment {
    pub(crate) fn enter() -> ComApartment {
        // S_OK and S_FALSE both need the matching `CoUninitialize`;
        // `RPC_E_CHANGED_MODE` (another model on this thread) does not.
        ComApartment(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}
