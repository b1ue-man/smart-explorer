//! The real sources of the system wallpaper (U34-4) and the entry points the
//! rest of the program uses: `platform::system_wallpaper(monitor)`, the note
//! for the settings and the poll that tells the surface the wallpaper changed
//! (`wallpaper_watch`).
use std::path::Path;

use super::gnome_settings::run_command;
use super::wallpaper::{self, Reading, RootImage, SettingsSource};
use super::wallpaper_root;
use crate::app::desktop_surface_host::SystemBackground;

pub(crate) struct SystemSource;

impl SettingsSource for SystemSource {
    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn gsettings(&self, schema: &str, key: &str) -> Option<String> {
        run_command(Path::new("gsettings"), &["get", schema, key])
    }

    fn read_file(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn xfconf(&self, channel: &str, property: &str) -> Option<String> {
        run_command(Path::new("xfconf-query"), &["-c", channel, "-p", property])
    }

    fn root_pixmap(&self, _monitor: &str) -> Result<RootImage, String> {
        wallpaper_root::save()
    }

    fn monitor_index(&self, monitor: &str) -> Option<usize> {
        super::monitors()
            .iter()
            .position(|known| known.key == monitor)
    }
}

/// The system wallpaper of `monitor` as the platform contract names it.
pub(crate) fn system_wallpaper(monitor: &str) -> Result<SystemBackground, String> {
    // Whoever reads it as a background wants to know when it changes.
    super::wallpaper_watch::watch_monitor(monitor);
    wallpaper::read(&SystemSource, monitor).background
}

/// For the settings hint: the path of the system wallpaper and the note
/// why it does not show as an image (a slideshow); an error says why the
/// source cannot be read. Does not start the watch of changes.
pub(crate) fn for_hint(
    monitor: &str,
) -> Result<(Option<std::path::PathBuf>, Option<String>), String> {
    let reading = wallpaper::read(&SystemSource, monitor);
    reading
        .background
        .map(|background| (background.wallpaper.path, reading.note))
}

/// What changes when the wallpaper changes, without rendering an image: the
/// settings read (as text), or the id of the root pixmap.
pub(super) fn signature(monitor: &str) -> String {
    let source = SystemSource;
    if wallpaper::uses_root_pixmap(&source) {
        return format!("root-pixmap:{:?}", wallpaper_root::pixmap_id());
    }
    let reading: Reading = wallpaper::read(&source, monitor);
    format!("{reading:?}")
}
