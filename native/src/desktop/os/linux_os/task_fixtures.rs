//! Task-test fixtures of the Linux traces (DSK-M3 signals): every folder
//! below a temporary root, snapshots of the places se Desktop may write,
//! planted fixed-name traces (a lost ledger) and foreign entries.
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::integration::LinuxPaths;
use crate::desktop::core::ledger::{AUTOSTART_FILE_NAME, GNOME_EXTENSION_UUID};

const RUNTIME_FILES: [&str; 4] = [
    "desktop.sock",
    "desktop.token",
    "desktop-gnome.json",
    "desktop-extension.json",
];
/// Entries of other programs below the same folders (F11, F12).
const FOREIGN: [(&str, &str); 5] = [
    (
        "config/autostart/other-app.desktop",
        "[Desktop Entry]\nName=Other\n",
    ),
    (
        "config/autostart/smart-explorer-sync-daemon.desktop",
        "[Desktop Entry]\nName=Smart Explorer Sync Daemon\nExec=\"/opt/se/se\" --sync-daemon\n",
    ),
    (
        "data/gnome-shell/extensions/ding@rastersoft.com/metadata.json",
        "{}",
    ),
    ("run/smart-explorer/Tresore/Privat/notiz.txt", "Tresor"),
    ("run/smart-explorer/drive.lock", "S2"),
];

pub(crate) fn paths(root: &Path) -> LinuxPaths {
    LinuxPaths {
        autostart_dir: root.join("config/autostart"),
        extensions_dir: root.join("data/gnome-shell/extensions"),
        runtime_dir: Some(root.join("run/smart-explorer")),
    }
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .display()
            .to_string();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            out.push(format!("dir {relative}"));
            walk(base, &path, out);
        } else {
            let bytes = std::fs::read(&path).unwrap_or_default();
            out.push(format!("file {relative} {:x}", Sha256::digest(&bytes)));
        }
    }
}

/// Every entry below `autostart/`, `gnome-shell/extensions/` and
/// `$XDG_RUNTIME_DIR/smart-explorer/` with a content hash.
pub(crate) fn trace_snapshot(root: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    walk(root, &root.join("config/autostart"), &mut entries);
    walk(
        root,
        &root.join("data/gnome-shell/extensions"),
        &mut entries,
    );
    walk(root, &root.join("run/smart-explorer"), &mut entries);
    entries.sort();
    entries
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// Every fixed-name trace, as a lost ledger would leave them behind.
pub(crate) fn plant_fixed_traces(root: &Path, gui: &Path) {
    let paths = paths(root);
    write(
        &paths.autostart_dir.join(AUTOSTART_FILE_NAME),
        &super::autostart::entry_for(gui).unwrap(),
    );
    let extension = paths.extensions_dir.join(GNOME_EXTENSION_UUID);
    write(&extension.join("metadata.json"), "{}");
    write(&extension.join("extension.js"), "// se Desktop\n");
    let runtime: PathBuf = paths.runtime_dir.unwrap();
    for name in RUNTIME_FILES {
        write(&runtime.join(name), "x");
    }
}

pub(crate) fn plant_foreign(root: &Path) {
    for (relative, text) in FOREIGN {
        write(&root.join(relative), text);
    }
}

/// Every foreign entry is still there with its content.
pub(crate) fn foreign_present(root: &Path) -> bool {
    FOREIGN.iter().all(|(relative, text)| {
        std::fs::read_to_string(root.join(relative)).is_ok_and(|found| found == *text)
    })
}

pub(crate) fn remove_foreign(_root: &Path) {}

/// The user switched the shell's login start off (`Hidden=true`).
pub(crate) fn switch_off_by_user(root: &Path, gui: &Path) {
    let entry = super::autostart::entry_for(gui).unwrap() + "Hidden=true\n";
    write(&paths(root).autostart_dir.join(AUTOSTART_FILE_NAME), &entry);
}

/// The switch-off as the user left it: the file content, untouched.
pub(crate) fn user_switch_off(root: &Path) -> Option<String> {
    std::fs::read_to_string(paths(root).autostart_dir.join(AUTOSTART_FILE_NAME))
        .ok()
        .filter(|body| body.contains("Hidden=true"))
}

pub(crate) fn undo_switch_off(_root: &Path) {}

/// The sync daemon's login start (F11) below `root`.
pub(crate) fn sync_autostart(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("config/autostart/smart-explorer-sync-daemon.desktop")).ok()
}

/// Linux has no Explorer context-menu entry (F10 applies to Windows).
pub(crate) fn context_menu_probe() -> Option<bool> {
    None
}

pub(crate) fn set_context_menu(_on: bool, _gui: &Path) {}
