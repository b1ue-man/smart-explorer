//! Login start of the se Desktop shell on Linux (C5.7): the XDG autostart
//! file `smart-explorer-desktop.desktop` with `Exec=<GUI> --desktop-shell`,
//! `TryExec=<GUI>` (Annahme A40: without the program the entry does nothing)
//! and `X-GNOME-Autostart-enabled=true`, the pattern of the sync daemon's
//! `smart-explorer-sync-daemon.desktop`. A switch-off by the user
//! (`Hidden=true`, `X-GNOME-Autostart-enabled=false`) is reported, and the
//! file then stays byte-identical.
use std::io;
use std::path::{Path, PathBuf};

use crate::desktop::core::ledger::AUTOSTART_FILE_NAME;

pub(crate) fn file_path(dir: &Path) -> PathBuf {
    dir.join(AUTOSTART_FILE_NAME)
}

/// Quoting of one `Exec` argument (Desktop Entry spec), byte-identical to
/// the rule of the sync daemon's autostart entry.
fn quote_exec_arg(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    for character in arg.chars() {
        match character {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(character);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn checked_path(gui: &Path) -> io::Result<&str> {
    let text = gui.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Pfad der Smart-Explorer-App ist kein UTF-8",
        )
    })?;
    if text.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Pfad der Smart-Explorer-App enthält ein Steuerzeichen",
        ));
    }
    Ok(text)
}

/// The file content for `gui`.
pub(crate) fn entry_for(gui: &Path) -> io::Result<String> {
    let path = checked_path(gui)?;
    let exec = format!("{} --desktop-shell", quote_exec_arg(path));
    let try_exec = path.replace('\\', "\\\\");
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=se Desktop\nComment=Start the Smart Explorer desktop surface at login\nExec={exec}\nTryExec={try_exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    ))
}

/// Writes the entry for `gui`; an entry the user switched off stays as it is.
pub(crate) fn write(dir: &Path, gui: &Path) -> io::Result<()> {
    if disabled_by_system(dir) {
        return Ok(());
    }
    let contents = entry_for(gui)?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(file_path(dir), contents)
}

/// Deletes exactly this module's file; every other entry stays.
pub(crate) fn remove(dir: &Path, name: &str) -> io::Result<()> {
    if name != AUTOSTART_FILE_NAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("fremder Autostart-Eintrag {name:?} wird nicht gelöscht"),
        ));
    }
    match std::fs::remove_file(file_path(dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn present(dir: &Path) -> bool {
    std::fs::symlink_metadata(file_path(dir)).is_ok()
}

/// The entry exists and the desktop was told to skip it.
pub(crate) fn disabled_by_system(dir: &Path) -> bool {
    std::fs::read_to_string(file_path(dir))
        .map(|body| system_disabled(&body))
        .unwrap_or(false)
}

/// `Hidden=true` or `X-GNOME-Autostart-enabled=false` in `[Desktop Entry]`.
fn system_disabled(body: &str) -> bool {
    let mut desktop = true;
    let mut disabled = false;
    for line in body.lines().map(str::trim) {
        if line.starts_with('[') {
            desktop = line == "[Desktop Entry]";
            continue;
        }
        if !desktop || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            match key.trim() {
                "Hidden" if value.eq_ignore_ascii_case("true") => disabled = true,
                "X-GNOME-Autostart-enabled" if value.eq_ignore_ascii_case("false") => {
                    disabled = true
                }
                _ => {}
            }
        }
    }
    disabled
}
