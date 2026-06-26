#[path = "shared/platform_helpers.rs"]
pub(in crate::app) mod shared_platform_helpers;

pub(in crate::app) use shared_platform_helpers::*;

use super::prelude::*;
use super::support_paths::OpenMode;
use super::*;
use shared_platform_helpers::{ClipboardEffect, ClipboardVirtualFile};

pub(in crate::app) fn confirm_yes_no(_title: &str, _msg: &str) -> bool {
    true
}

pub(in crate::app) fn list_drives() -> Vec<String> {
    vec!["/".to_string()]
}

pub(in crate::app) fn drive_info_list(_drives: &[String]) -> Vec<(String, u64, u64)> {
    Vec::new()
}

pub(in crate::app) fn reveal_path_in_file_manager(path: &str) {
    let target = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(path));
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

pub(in crate::app) fn open_local_path(path: &str, _mode: OpenMode) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
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

pub(in crate::app) fn spawn_updated_app(exe: &Path) {
    let _ = std::process::Command::new(exe).arg("--updated").spawn();
}

pub(in crate::app) fn update_payload_name() -> &'static str {
    "smart_explorer"
}

pub(in crate::app) fn shell_integration_available() -> bool {
    false
}

pub(in crate::app) fn set_context_menu_enabled(_on: bool) -> std::io::Result<()> {
    Ok(())
}

pub(in crate::app) fn show_properties_for_path(_path: &str) {}

pub(in crate::app) fn write_clipboard_files(
    _paths: &[String],
    _effect: ClipboardEffect,
) -> Result<(), String> {
    Err("Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string())
}

pub(in crate::app) fn read_clipboard_files() -> Option<(Vec<String>, bool)> {
    None
}

pub(in crate::app) fn set_virtual_clipboard(
    _files: Vec<ClipboardVirtualFile>,
) -> Result<u32, String> {
    Err("Virtuelle Datei-Zwischenablage ist auf dieser Plattform nicht verfuegbar".to_string())
}

pub(in crate::app) fn virtual_clipboard_sequence() -> Option<u32> {
    None
}

pub(in crate::app) fn clipboard_file_ops_supported() -> bool {
    false
}

pub(in crate::app) fn drag_out_files(_files: &[String]) {}

pub(in crate::app) fn os_drag_out_supported() -> bool {
    false
}

pub(in crate::app) fn available_space_for_path(_path: &Path) -> Option<u64> {
    None
}

pub(in crate::app) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(src, dest)
}

pub(in crate::app) fn process_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    Path::new("/proc").join(pid.to_string()).exists()
}

impl App {
    pub(in crate::app) fn show_shell_menu_for(&mut self, clicked_path: &str, _ctx: &egui::Context) {
        self.open_in_explorer(clicked_path);
    }

    pub(in crate::app) fn show_background_menu(&mut self) {}

    pub(in crate::app) fn start_clip_key_poller(&mut self, _ctx: &egui::Context) {}

    pub(in crate::app) fn start_watcher(&mut self) {}

    pub(in crate::app) fn drain_watcher(&mut self) {}

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
