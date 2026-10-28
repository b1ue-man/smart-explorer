#[path = "windows/media_launch.rs"]
mod media_launch;
#[path = "windows/platform.rs"]
mod platform;
#[path = "shared/platform_helpers.rs"]
pub(in crate::app) mod shared_platform_helpers;
#[path = "windows/shell_menus.rs"]
mod shell_menus;
#[path = "windows/watchers.rs"]
mod watchers;

pub(in crate::app) use platform::*;
pub(in crate::app) use shared_platform_helpers::*;

#[path = "windows/drive_events.rs"]
mod drive_events;
#[path = "windows/drive_inventory.rs"]
mod drive_inventory;
pub(crate) use drive_events::start_drive_events;
pub(crate) use drive_inventory::{read_drive_capacity, read_drive_inventory};

/// Windows has no X11 window type.
pub(in crate::app) fn indicator_window_type() -> Option<eframe::egui::X11WindowType> {
    None
}

/// UI storage uses slash separators only on Windows; Unix backslashes are names.
pub(in crate::app) fn local_path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Explorer's icon of a local file, folder or shortcut (square RGBA, side).
pub(in crate::app) fn shell_icon_rgba(path: &std::path::Path) -> Option<(u32, Vec<u8>)> {
    crate::screen::os::app_icons::path_icon_rgba(path)
}

/// Windows' stock icon for the first of `names` (freedesktop icon names).
pub(in crate::app) fn named_icon_rgba(names: &[&str]) -> Option<(u32, Vec<u8>)> {
    crate::screen::os::app_icons::named_icon_rgba(names)
}
