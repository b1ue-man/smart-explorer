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
pub mod desktop_folder;
#[path = "instance.rs"]
pub mod instance;
#[path = "integration.rs"]
mod integration;
#[path = "jump_list.rs"]
pub mod jump_list;
#[path = "monitors.rs"]
pub mod monitors;
#[path = "nav_pane.rs"]
pub mod nav_pane;
#[path = "process.rs"]
pub mod process;
#[cfg(test)]
#[path = "registry_task_tests.rs"]
mod registry_task_tests;
#[path = "shell_actions.rs"]
mod shell_actions;
#[path = "shell_parts.rs"]
mod shell_parts;
#[path = "surface_window.rs"]
pub mod surface_window;
#[path = "system_items.rs"]
pub mod system_items;
#[cfg(test)]
#[path = "task_fixtures.rs"]
pub mod task_fixtures;
#[path = "wallpaper.rs"]
pub mod wallpaper;
#[cfg(test)]
#[path = "window_task_env.rs"]
mod window_task_env;
#[cfg(test)]
#[path = "window_task_report.rs"]
mod window_task_report;
#[cfg(test)]
#[path = "window_task_tests.rs"]
mod window_task_tests;

pub use desktop_folder::{desktop_folders, hidden_entry};
pub use integration::WindowsIntegration as PlatformIntegration;
pub use jump_list::clear_here as clear_jump_list_here;
pub use monitors::{monitor_names, monitors, work_area};
pub use shell_actions::{pointer_outside, release_pointer, shell_action};
pub use shell_parts::ShellParts;
pub use surface_window::attach_surface;
pub use system_items::initial_system_items;
pub use wallpaper::system_wallpaper;

use crate::desktop::core::mechanism::{DisplayServer, SessionFacts};
use crate::desktop::core::state::Platform;

pub const PLATFORM: Platform = Platform::Windows;
/// A halted shell stays while `notify_desktop` shows its balloon (30 s).
pub const NOTICE_LINGER: std::time::Duration = std::time::Duration::from_secs(31);

/// Names of the GUI program (`updater` installs `Smart Explorer.exe`;
/// development builds run `smart_explorer.exe`).
const GUI_IMAGES: [&str; 2] = ["Smart Explorer.exe", "smart_explorer.exe"];

pub fn is_gui_image(exe: &Path) -> bool {
    exe.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| GUI_IMAGES.iter().any(|gui| gui.eq_ignore_ascii_case(name)))
}

/// Windows always offers the lowest tool window (C1.2).
pub fn session_facts() -> SessionFacts {
    SessionFacts {
        display: DisplayServer::Windows,
        ..SessionFacts::default()
    }
}

/// COM on the calling thread for one call: a single-threaded apartment
/// when the thread has none; an apartment the thread already has (winit's
/// OLE on a surface's UI thread) stays as it is.
pub struct ComApartment(bool);

impl ComApartment {
    pub fn enter() -> ComApartment {
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
