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

/// UI storage uses slash separators only on Windows; Unix backslashes are names.
pub(in crate::app) fn local_path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
