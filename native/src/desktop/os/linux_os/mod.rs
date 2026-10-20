//! Linux adapter of se Desktop (DSK-M3, DSK-M6 to DSK-M8): login start, single
//! instance, processes, registrations and the facts the core decides on
//! (`session_facts`), the X11 surface (`x11_surface`, `x11_monitors`,
//! `x11_clipboard`), the system wallpaper (`wallpaper*`), the desktop folder
//! and its starters (`desktop_folder`, `desktop_entry_trust`,
//! `open_with_portal`), the Wayland layer backend (`layer_shell`) and the
//! GNOME Shell extension (`gnome_*`).
use std::path::Path;

#[path = "autostart.rs"]
mod autostart;
#[path = "clipboard_formats.rs"]
mod clipboard_formats;
#[path = "desktop_entry_trust.rs"]
mod desktop_entry_trust;
#[path = "desktop_folder.rs"]
mod desktop_folder;
#[path = "gnome_channel.rs"]
pub(crate) mod gnome_channel;
#[path = "gnome_control.rs"]
pub(crate) mod gnome_control;
#[path = "gnome_dbus.rs"]
pub(crate) mod gnome_dbus;
#[path = "gnome_extension.rs"]
pub(crate) mod gnome_extension;
#[path = "gnome_settings.rs"]
pub(crate) mod gnome_settings;
#[path = "gnome_steward.rs"]
pub(crate) mod gnome_steward;
#[path = "instance.rs"]
pub(crate) mod instance;
#[path = "integration.rs"]
pub(crate) mod integration;
// DSK-M7: the own egui host on `wlr-layer-shell` (one process, every output).
#[cfg(test)]
#[path = "gnome_bus_tests.rs"]
mod gnome_bus_tests;
#[cfg(test)]
#[path = "gnome_extension_tests.rs"]
mod gnome_extension_tests;
#[cfg(test)]
#[path = "gnome_headless_tests.rs"]
mod gnome_headless_tests;
#[cfg(test)]
#[path = "gnome_steward_tests.rs"]
mod gnome_steward_tests;
#[cfg(test)]
#[path = "gnome_task_tests.rs"]
mod gnome_task_tests;
#[cfg(test)]
#[path = "gnome_test_support.rs"]
mod gnome_test_support;
#[path = "layer_shell/mod.rs"]
pub(crate) mod layer_shell;
#[path = "monitor_keys.rs"]
mod monitor_keys;
#[path = "open_with_portal.rs"]
mod open_with_portal;
#[path = "probe_timeout.rs"]
mod probe_timeout;
#[path = "process.rs"]
pub(crate) mod process;
#[path = "session_facts.rs"]
pub(crate) mod session_facts;
#[cfg(test)]
#[path = "task_fixtures.rs"]
pub(crate) mod task_fixtures;
#[cfg(test)]
#[path = "trust_tests.rs"]
mod trust_tests;
#[path = "wallpaper.rs"]
mod wallpaper;
#[path = "wallpaper_kde.rs"]
mod wallpaper_kde;
#[path = "wallpaper_root.rs"]
mod wallpaper_root;
#[path = "wallpaper_system.rs"]
mod wallpaper_system;
#[path = "wallpaper_watch.rs"]
mod wallpaper_watch;
#[path = "x11_clipboard.rs"]
mod x11_clipboard;
#[path = "x11_facts.rs"]
mod x11_facts;
#[path = "x11_monitors.rs"]
mod x11_monitors;
#[path = "x11_surface.rs"]
mod x11_surface;
#[cfg(test)]
#[path = "x11_task_env.rs"]
mod x11_task_env;
#[cfg(test)]
#[path = "x11_task_surface.rs"]
mod x11_task_surface;

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

/// What the session offers (C2.2 inputs, `session_facts`): the display
/// server, the window manager's atoms on X11, the layer protocol on Wayland,
/// the GNOME Shell. Every probe runs under a limit of two seconds.
pub(crate) fn session_facts() -> SessionFacts {
    session_facts::probe(&session_facts::SystemProbe)
}

/// The display server from the environment alone (no probe).
fn display_server() -> DisplayServer {
    session_facts::display_server(&session_facts::SystemProbe)
}

/// Linux has no "Papierkorb"/"Dieser PC" items; the se defaults are stored.
pub(crate) fn initial_system_items() -> SystemItems {
    SystemItems::initial(None, None)
}

/// The monitors of the session: X11 RandR monitors (DSK-M6, each with its own
/// surface process), the logical monitors of Mutter under GNOME (DSK-M8, the
/// extension starts one surface per monitor) or the Wayland outputs (DSK-M7;
/// the layer backend serves all of them from one process). No display server,
/// no monitors.
pub(crate) fn monitors() -> Vec<Monitor> {
    match display_server() {
        DisplayServer::X11 => x11_monitors::monitors(),
        DisplayServer::Wayland => wayland_monitors(),
        DisplayServer::Windows | DisplayServer::Headless => Vec::new(),
    }
}

fn gnome_session() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|value| {
        value
            .split(':')
            .any(|part| part.trim().eq_ignore_ascii_case("GNOME"))
    })
}

/// The last list Mutter gave; a question that fails for a moment (the bus is
/// busy) must not look like a change of the monitors, which would restart
/// every surface.
static LAST_GNOME: std::sync::Mutex<Option<Vec<Monitor>>> = std::sync::Mutex::new(None);

/// The logical monitors of Mutter in the order of its call; one monitor
/// `gnome-0` when it names none or cannot be asked and nothing is known yet
/// (U34-14).
fn gnome_monitors() -> Vec<Monitor> {
    let mut last = LAST_GNOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fallback = || {
        vec![Monitor {
            key: "gnome-0".into(),
            primary: true,
            rect: None,
        }]
    };
    match gnome_dbus::display_monitors() {
        Some(list) if !list.is_empty() => {
            let monitors: Vec<Monitor> = list
                .into_iter()
                .map(|monitor| Monitor {
                    key: monitor.key,
                    primary: monitor.primary,
                    rect: Some(monitor.rect),
                })
                .collect();
            *last = Some(monitors.clone());
            monitors
        }
        Some(_) => fallback(),
        None => last.clone().unwrap_or_else(fallback),
    }
}

/// The outputs the compositor announces, in its logical coordinates (DSK-M7,
/// `layer_shell::registry`); none when the compositor cannot be asked.
fn wayland_monitors() -> Vec<Monitor> {
    if gnome_session() {
        return gnome_monitors();
    }
    layer_shell::registry::probe_outputs()
        .map(|outputs| outputs.into_iter().map(Monitor::from).collect())
        .unwrap_or_default()
}

/// Linux has no jump list.
pub(crate) fn clear_jump_list_here() -> Result<(), String> {
    Ok(())
}

/// Puts the surface window under the others (X11, `x11_surface`); other
/// windows are left as they are.
pub(crate) fn attach_surface(
    window: crate::app::desktop_surface_host::NativeWindow,
    monitor: &str,
    signals: std::sync::Arc<crate::app::desktop_surface_host::SurfaceSignals>,
) -> Result<(), String> {
    x11_surface::attach_surface(window, monitor, signals)
}

/// The monitor without panels and docks (X11 `_NET_WORKAREA`); the whole
/// monitor elsewhere (a layer surface leaves the panels alone itself).
pub(crate) fn work_area(monitor: &str) -> Option<[i32; 4]> {
    (display_server() == DisplayServer::X11)
        .then(|| x11_surface::work_area(monitor))
        .flatten()
}

/// `(key, label)` of every monitor for the menu "Auf Bildschirm verschieben".
pub(crate) fn monitor_names() -> Vec<(String, String)> {
    if display_server() == DisplayServer::X11 {
        return x11_monitors::monitor_names();
    }
    monitors()
        .into_iter()
        .map(|monitor| (monitor.key.clone(), monitor.key))
        .collect()
}

/// The system wallpaper of a monitor (`wallpaper`, C2.6).
pub(crate) fn system_wallpaper(
    monitor: &str,
) -> Result<crate::app::desktop_surface_host::SystemBackground, String> {
    wallpaper_system::system_wallpaper(monitor)
}

/// The path of the system wallpaper and why it does not show as an image,
/// for the settings hint (`wallpaper_system::for_hint`).
pub(crate) fn system_wallpaper_for_hint(
    monitor: &str,
) -> Result<(Option<std::path::PathBuf>, Option<String>), String> {
    wallpaper_system::for_hint(monitor)
}

/// The system wallpaper of `monitor` changed since the last call (a poll of
/// the source every five seconds).
pub(crate) fn wallpaper_changed(monitor: &str) -> bool {
    wallpaper_watch::changed(monitor)
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

/// The desktop folder of `XDG_DESKTOP_DIR` (`desktop_folder`).
pub(crate) fn desktop_folders() -> Vec<super::feed_sources::DesktopFolder> {
    desktop_folder::desktop_folders()
}

/// The note while the desktop folder is switched off.
pub(crate) fn desktop_folder_notice() -> Option<&'static str> {
    desktop_folder::folder_notice()
}

/// Linux hides by the leading dot (`feed_sources::scan_folders`).
pub(crate) fn hidden_entry(_entry: &std::fs::DirEntry) -> bool {
    false
}

/// "Öffnen mit …" and starters (`open_with_portal`); `true` when handled.
pub(crate) fn open_desktop_path(path: &Path, choose: bool) -> bool {
    open_with_portal::open_desktop_path(path, choose)
}

/// The `extension_state` of the status (`gnome_extension`).
pub(crate) fn extension_state(facts: &SessionFacts) -> Option<&'static str> {
    gnome_extension::current_state(facts)
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
