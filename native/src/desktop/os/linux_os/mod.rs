//! Linux adapter of se Desktop (DSK-M3): login start, single instance,
//! processes, registrations and the facts the core decides on. The session
//! probes (`_NET_SUPPORTED`, layer-shell, GNOME version, RandR monitors)
//! follow with DSK-M6 in `session_facts.rs`; until then only the
//! environment is known, so the core reports the surface as unavailable.
use std::path::Path;

#[path = "autostart.rs"]
mod autostart;
#[path = "gnome_settings.rs"]
pub(crate) mod gnome_settings;
#[path = "instance.rs"]
pub(crate) mod instance;
#[path = "integration.rs"]
pub(crate) mod integration;
#[path = "process.rs"]
pub(crate) mod process;
#[cfg(test)]
#[path = "task_fixtures.rs"]
pub(crate) mod task_fixtures;

pub(crate) use integration::LinuxIntegration as PlatformIntegration;

use super::Monitor;
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::mechanism::{DisplayServer, SessionFacts};
use crate::desktop::core::state::Platform;

pub(crate) const PLATFORM: Platform = Platform::Linux;
/// A halted shell stays until the notification went out on the session bus.
pub(crate) const NOTICE_LINGER: std::time::Duration = std::time::Duration::from_secs(3);

/// Name of the GUI program (`install-linux.sh`, development builds).
const GUI_IMAGE: &str = "smart_explorer";

pub(crate) fn is_gui_image(exe: &Path) -> bool {
    exe.file_name().is_some_and(|name| name == GUI_IMAGE)
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// What the environment tells about the session (C2.2 inputs). The window
/// manager and compositor probes are DSK-M6.
pub(crate) fn session_facts() -> SessionFacts {
    let display = if env_set("WAYLAND_DISPLAY") {
        DisplayServer::Wayland
    } else if env_set("DISPLAY") {
        DisplayServer::X11
    } else {
        DisplayServer::Headless
    };
    let current_desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|value| {
            value
                .split(':')
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    SessionFacts {
        display,
        current_desktop,
        ..SessionFacts::default()
    }
}

/// Linux has no "Papierkorb"/"Dieser PC" items; the se defaults are stored.
pub(crate) fn initial_system_items() -> SystemItems {
    SystemItems::initial(None, None)
}

/// Monitors with their own surface process; X11 RandR monitors follow with
/// DSK-M6, the layer backend serves every output from one process (DSK-M7).
pub(crate) fn monitors() -> Vec<Monitor> {
    Vec::new()
}

/// Linux has no jump list.
pub(crate) fn clear_jump_list_here() -> Result<(), String> {
    Ok(())
}

// DSK-M5 (U32): the names the shared code and the surface host call on
// every platform. Linux fills them with DSK-M6 (U34: X11 window, work area,
// wallpaper, `XDG_DESKTOP_DIR`) and DSK-M7 (U37); until then the surface
// has no window adapter, the whole monitor as work area, no monitor names,
// no system wallpaper and no Explorer actions, and the shell has no parts
// besides the surfaces.
pub(crate) fn attach_surface(
    _window: crate::app::desktop_surface_host::NativeWindow,
    _monitor: &str,
    _signals: std::sync::Arc<crate::app::desktop_surface_host::SurfaceSignals>,
) -> Result<(), String> {
    Ok(())
}

pub(crate) fn work_area(_monitor: &str) -> Option<[i32; 4]> {
    None
}

pub(crate) fn monitor_names() -> Vec<(String, String)> {
    Vec::new()
}

pub(crate) fn system_wallpaper(
    _monitor: &str,
) -> Result<crate::app::desktop_surface_host::SystemBackground, String> {
    Err(crate::app::desktop_surface_host::NO_SYSTEM_BACKGROUND.into())
}

pub(crate) fn shell_action(
    _action: crate::app::desktop_surface_host::ShellAction<'_>,
    _heartbeat: &mut dyn FnMut(),
) -> Result<crate::app::desktop_surface_host::ShellOutcome, String> {
    Err("Explorer-Aktionen gibt es nur unter Windows".into())
}

pub(crate) fn pointer_outside(_window: crate::app::desktop_surface_host::NativeWindow) -> bool {
    false
}

pub(crate) fn release_pointer(_window: crate::app::desktop_surface_host::NativeWindow) {}

/// Only the suite switch names a desktop folder until DSK-M6.
pub(crate) fn desktop_folders() -> Vec<super::feed_sources::DesktopFolder> {
    Vec::new()
}

/// Linux hides by the leading dot (`feed_sources::scan_folders`).
pub(crate) fn hidden_entry(_entry: &std::fs::DirEntry) -> bool {
    false
}

/// Linux has no navigation pane and no jump list.
pub(crate) struct ShellParts;

impl ShellParts {
    pub(crate) fn new(_dir: super::store::DesktopDir, _gui: std::path::PathBuf) -> ShellParts {
        ShellParts
    }

    pub(crate) fn sync(
        &mut self,
        _state: &crate::desktop::core::state::ModuleState,
        _tree: &super::feed_sources::DeviceTreeView,
        _tiles: Option<&crate::desktop::core::tiles::TileModel>,
    ) {
    }
}
