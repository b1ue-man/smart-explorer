//! „Im Dateimanager öffnen“ for the drive window and the settings (DRV-M5,
//! DRV-M6), and the one-time hint after the device tree appeared by itself
//! (decision C5).
//!
//! Linux asks the file manager of the session (`FileManager1`, else
//! `xdg-open`, `mount::os::linux_os::show_folders`), Windows opens the drive
//! in Explorer. Both run on the thread of `spawn_mount_action`, never on the
//! interface thread.
use crate::app::mount_ui::MountUiResult;
use crate::app::prelude::*;
use crate::app::App;
use crate::mount::{MountSnapshot, MountStatus};

/// The label of the button.
pub(in crate::app) fn open_button_label() -> &'static str {
    if cfg!(windows) {
        "Im Explorer öffnen"
    } else {
        "Im Dateimanager öffnen"
    }
}

/// The folder a mounted drive shows: `X:\` or the mount folder.
pub(in crate::app) fn mounted_location(status: &MountStatus) -> Option<PathBuf> {
    match status {
        MountStatus::Mounted { drive } => Some(PathBuf::from(format!("{drive}\\"))),
        MountStatus::MountedAt { path } => Some(path.clone()),
        _ => None,
    }
}

impl App {
    /// Shows `location` in the file manager on a background thread; an error
    /// appears as „Laufwerk: …“.
    pub(in crate::app) fn open_in_file_manager(&mut self, location: PathBuf) {
        self.spawn_mount_action("Laufwerk wird geöffnet", move || {
            show(&location).map(|()| MountUiResult::Opened)
        });
    }
}

/// The same for a folder given as text (the status of the device tree).
pub(in crate::app) fn open_in_file_manager_path(app: &mut App, location: &str) {
    app.open_in_file_manager(PathBuf::from(location));
}

#[cfg(target_os = "linux")]
fn show(location: &Path) -> Result<(), String> {
    crate::mount::os::linux_os::show_folders(&[location.to_path_buf()])
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn show(location: &Path) -> Result<(), String> {
    crate::app::platform_helpers::open_local_path(
        &location.to_string_lossy(),
        crate::app::support_paths::OpenMode::Default,
    );
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn show(_location: &Path) -> Result<(), String> {
    Err("Laufwerke gibt es nur unter Windows und Linux".into())
}

/// The one-time hint, once the device tree is mounted and the hint was not
/// shown yet; the file that remembers it is written here. It is also
/// written when the user switches the tree on himself (no hint then).
pub(in crate::app) fn devices_hint(mounts: &[MountSnapshot]) -> Option<String> {
    let tree = mounts
        .iter()
        .find(|mount| mount.config.source.is_device_tree())?;
    let text = match &tree.status {
        MountStatus::Mounted { drive } => format!(
            "Verbundene Geräte erscheinen jetzt als Laufwerk {drive} („{}“). Ausschalten oder \
             Buchstaben ändern: Einstellungen → Integration → Verbundene Geräte als Ordner.",
            crate::mount::devices_label()
        ),
        MountStatus::MountedAt { path } => format!(
            "Verbundene Geräte erscheinen jetzt im Ordner {}. Ausschalten: Einstellungen → \
             Integration → Verbundene Geräte als Ordner.",
            path.display()
        ),
        _ => return None,
    };
    if !crate::daemon::devices_hint_due() {
        return None;
    }
    crate::daemon::devices_hint_done();
    Some(text)
}
