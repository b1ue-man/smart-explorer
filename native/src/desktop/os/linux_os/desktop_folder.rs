//! The desktop folder on Linux (U34-5, C2.7): `XDG_DESKTOP_DIR` of
//! `$XDG_CONFIG_HOME/user-dirs.dirs` (`$HOME` replaced, quotes removed),
//! `~/Desktop` without that line. When the value is the home folder itself,
//! the desktop folder counts as switched off (the xdg-user-dirs convention
//! of a user who wants no desktop files): no folder, and the note
//! [`HOME_NOTICE`]. The folder is never created. Linux hides an entry by its
//! leading dot (`feed_sources::scan_folders`), so `hidden_entry` stays false.
use std::path::{Path, PathBuf};

use crate::desktop::os::feed_sources::DesktopFolder;

/// The status note while the desktop folder is switched off.
pub(crate) const HOME_NOTICE: &str =
    "Desktop-Dateien sind abgeschaltet (XDG_DESKTOP_DIR zeigt auf den Home-Ordner)";

/// What `user-dirs.dirs` makes of the desktop folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolved {
    Folder(PathBuf),
    /// The value is the home folder.
    Off,
}

/// The value of `XDG_DESKTOP_DIR` in the text of `user-dirs.dirs`.
fn desktop_value(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("XDG_DESKTOP_DIR="))?;
    let value = line["XDG_DESKTOP_DIR=".len()..].trim();
    let value = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(value);
    let home_text = home.to_string_lossy();
    let replaced = value
        .replace("${HOME}", &home_text)
        .replace("$HOME", &home_text);
    let path = PathBuf::from(replaced);
    path.is_absolute().then_some(path)
}

fn same_folder(left: &Path, right: &Path) -> bool {
    let trim = |path: &Path| {
        let text = path.to_string_lossy().into_owned();
        let trimmed = text.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }
    };
    trim(left) == trim(right)
}

/// The desktop folder for `user_dirs` (the text of the file, when there is
/// one) and `home`.
pub(crate) fn resolve(user_dirs: Option<&str>, home: &Path) -> Resolved {
    let folder = user_dirs
        .and_then(|text| desktop_value(text, home))
        .unwrap_or_else(|| home.join("Desktop"));
    if same_folder(&folder, home) {
        Resolved::Off
    } else {
        Resolved::Folder(folder)
    }
}

fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// `resolve` for this user's files.
pub(crate) fn current() -> Option<Resolved> {
    let home = absolute_env("HOME")?;
    let config = absolute_env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).ok();
    Some(resolve(text.as_deref(), &home))
}

/// The folders for a resolution: the one folder, none while it is switched
/// off or the home folder is unknown.
pub(crate) fn folders_of(resolved: Option<Resolved>) -> Vec<DesktopFolder> {
    match resolved {
        Some(Resolved::Folder(path)) => vec![DesktopFolder {
            path,
            public: false,
            launchers: true,
        }],
        Some(Resolved::Off) | None => Vec::new(),
    }
}

/// The desktop folders of this user.
pub(crate) fn desktop_folders() -> Vec<DesktopFolder> {
    folders_of(current())
}

/// The note for status and settings while the desktop folder is off.
pub(crate) fn folder_notice() -> Option<&'static str> {
    matches!(current(), Some(Resolved::Off)).then_some(HOME_NOTICE)
}
