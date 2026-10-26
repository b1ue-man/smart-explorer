#[path = "shared/platform_helpers.rs"]
pub(in crate::app) mod shared_platform_helpers;

pub(in crate::app) use shared_platform_helpers::*;

use super::prelude::*;
use super::support_paths::OpenMode;
use super::*;

pub(in crate::app) fn confirm_yes_no(title: &str, msg: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(msg)
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

pub(in crate::app) fn list_drives() -> Vec<String> {
    read_drive_inventory().map(|entries| entries.into_iter().map(|entry| entry.root).collect())
        .unwrap_or_else(|_| vec!["/".to_string()])
}

pub(in crate::app) fn drive_info_list(drives: &[String]) -> Vec<(String, u64, u64)> {
    drives.iter().map(|root| {
        let (free, total) = drive_inventory::read_drive_capacity(root);
        (root.clone(), free, total)
    }).collect()
}

pub(in crate::app) fn reveal_path_in_file_manager(path: &str) {
    let target = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(path));
    spawn_xdg_open(target);
}

pub(in crate::app) fn open_local_path(path: &str, _mode: OpenMode) {
    spawn_xdg_open(Path::new(path));
}

/// Starts `xdg-open` without inheriting this process's stdio (the se Desktop
/// surface uses stdin/stdout as its channel to the supervisor) and reaps it on
/// a short-lived thread, so no zombie process stays behind.
fn spawn_xdg_open(target: &Path) {
    let spawned = std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        let _ = std::thread::Builder::new()
            .name("xdg-open-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });
    }
}

pub(in crate::app) struct EditProcess;

impl EditProcess {
    pub(in crate::app) fn is_finished(&self) -> bool {
        true
    }
}

pub(in crate::app) fn launch_local_for_edit(path: &str, mode: OpenMode) -> Option<EditProcess> {
    open_local_path(path, mode);
    None
}

pub(in crate::app) fn open_terminal_at(path: &str) {
    if path.is_empty() {
        return;
    }
    let _ = std::process::Command::new("x-terminal-emulator")
        .current_dir(path)
        .spawn();
}

pub(in crate::app) fn spawn_updated_app(exe: &Path) -> std::io::Result<()> {
    std::process::Command::new(exe)
        .arg("--updated")
        .spawn()
        .map(|_| ())
}

pub(in crate::app) fn update_payload_name() -> &'static str {
    "smart_explorer"
}

pub(in crate::app) fn shell_integration_available() -> bool {
    false
}

/// Network shares with WNet login (UNC) exist only on Windows; Linux reaches
/// SMB servers through the SMB protocol of the connection dialog.
pub(in crate::app) fn unc_connections_available() -> bool {
    false
}

/// Child processes of the GUI need no console handling on Linux.
pub(in crate::app) fn hide_child_console(_command: &mut std::process::Command) {}

pub(in crate::app) fn set_context_menu_enabled(_on: bool) -> std::io::Result<()> {
    Ok(())
}

pub(in crate::app) fn show_properties_for_path(_path: &str) {}

/// A surface process of se Desktop installs a provider (DSK-M7: Wayland
/// `wl_data_device`); every other process has none and keeps the answer
/// "not available".
pub(in crate::app) fn write_clipboard_files(
    paths: &[String],
    effect: ClipboardEffect,
) -> Result<(), String> {
    let Some(provider) = super::desktop_surface_clipboard::installed() else {
        return Err("Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string());
    };
    let files: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).collect();
    provider.write(&files, matches!(effect, ClipboardEffect::Move))
}

pub(in crate::app) fn write_clipboard_files_if_sequence(
    _paths: &[String],
    _effect: ClipboardEffect,
    _expected: u32,
) -> Result<Option<u32>, String> {
    Err("Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string())
}

pub(in crate::app) fn read_clipboard_files() -> Result<Option<(Vec<String>, bool)>, String> {
    let Some(provider) = super::desktop_surface_clipboard::installed() else {
        return Err("Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string());
    };
    Ok(provider.read()?.map(|(files, cut)| {
        let paths = files
            .into_iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect();
        (paths, cut)
    }))
}

pub(in crate::app) fn set_virtual_clipboard_if_sequence(
    _files: Vec<ClipboardVirtualFile>,
    _expected: u32,
) -> Result<Option<u32>, String> {
    Err("Virtuelle Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string())
}

pub(in crate::app) fn virtual_clipboard_sequence() -> Option<u32> {
    None
}

pub(in crate::app) fn clipboard_file_ops_supported() -> bool {
    super::desktop_surface_clipboard::installed().is_some()
}

/// No desktop file-clipboard protocol is spoken here; the app keeps its own.
pub(in crate::app) fn set_remote_clipboard(
    _source: crate::transfer::SelectionSource,
) -> Result<u32, String> {
    Err("Virtuelle Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string())
}

pub(in crate::app) fn drag_out_remote(
    _source: crate::transfer::SelectionSource,
) -> Result<crate::dragout::DragOutOutcome, String> {
    Err("OS drag-and-drop is not available on this platform".to_string())
}

pub(in crate::app) fn remote_clipboard_supported() -> bool {
    false
}

pub(in crate::app) fn drag_out_files(
    _files: &[String],
) -> Result<crate::dragout::DragOutOutcome, String> {
    Err("OS drag-and-drop is not available on this platform".to_string())
}

/// Unix names are stored and addressed literally; Win32 name rules only
/// matter for the optional problem-names filter here.
pub(in crate::app) fn local_names_follow_win32_rules() -> bool {
    false
}

pub(in crate::app) fn os_drag_out_supported() -> bool {
    false
}

pub(in crate::app) use crate::transfer::{replace_file_atomic, upload_is_link_like};

pub(in crate::app) fn process_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    match std::fs::metadata(Path::new("/proc").join(pid.to_string())) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

impl App {
    pub(in crate::app) fn show_shell_menu_for(&mut self, clicked_path: &str, _ctx: &egui::Context) {
        self.open_in_explorer(clicked_path);
    }

    pub(in crate::app) fn show_background_menu(&mut self) {}

    pub(in crate::app) fn start_clip_key_poller(&mut self, _ctx: &egui::Context) {}

    pub(in crate::app) fn start_watcher(&mut self) {}

    pub(in crate::app) fn drain_watcher(&mut self) {}

    pub(in crate::app) fn update_drive_watches(&mut self, _removed: &[String], _added: &[String]) {}

    pub(in crate::app) fn initial_context_menu_enabled() -> bool {
        false
    }

    pub(in crate::app) fn should_start_watcher(&self) -> bool {
        false
    }

    pub(in crate::app) fn should_start_clip_key_poller(&self) -> bool {
        false
    }
}

#[path = "linux_os/drive_events.rs"]
mod drive_events;
#[path = "linux_os/drive_inventory.rs"]
mod drive_inventory;
pub(crate) use drive_events::start_drive_events;
pub(crate) use drive_inventory::{read_drive_inventory, read_drive_capacity};

pub(in crate::app) fn local_path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}
