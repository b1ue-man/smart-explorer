//! Running apps from the systemd user units that desktops start them in
//! (SCR-M5, U19-7, assumption A7), for desktops with no window-list protocol
//! (GNOME). The unit naming follows systemd's desktop-environment convention:
//! a scope is `app[-<launcher>]-<ApplicationID>-<RANDOM>.scope`, a service
//! `app[-<launcher>]-<ApplicationID>[@<RANDOM>].service`. The app id is
//! matched with the desktop files for its display name. The list has apps and
//! no windows.
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::screen::core::window_list::{AppEntry, ListSource, WindowList};

/// The starters whose name precedes the application id in a unit name.
const LAUNCHERS: [&str; 4] = ["gnome-", "KDE-", "kde-", "xdg-"];

/// One app unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppUnit {
    pub launcher: Option<String>,
    pub app_id: String,
}

/// Undoes systemd's `\xNN` escaping of a unit name part.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = bytes
            .get(at..at + 4)
            .filter(|chunk| chunk.starts_with(b"\\x"));
        let value = escaped
            .and_then(|chunk| std::str::from_utf8(&chunk[2..]).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match value {
            Some(byte) => {
                out.push(byte);
                at += 4;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The launcher and application id of the unit `name`, or `None` when it is
/// no app unit.
pub(crate) fn parse_app_unit(name: &str) -> Option<AppUnit> {
    let (stem, scope) = match name.strip_suffix(".scope") {
        Some(stem) => (stem, true),
        None => (name.strip_suffix(".service")?, false),
    };
    let rest = stem.strip_prefix("app-")?;
    let rest = if scope {
        // The part after the last `-` is the random number.
        let (rest, random) = rest.rsplit_once('-')?;
        if random.is_empty() || !random.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        rest
    } else {
        rest.split_once('@').map_or(rest, |(rest, _)| rest)
    };
    let launcher = LAUNCHERS.iter().find(|prefix| rest.starts_with(**prefix));
    let (launcher, app_id) = match launcher {
        Some(prefix) => (
            Some(prefix.trim_end_matches('-').to_string()),
            &rest[prefix.len()..],
        ),
        None => (None, rest),
    };
    (!app_id.is_empty()).then(|| AppUnit {
        launcher,
        app_id: unescape(app_id),
    })
}

/// The directories with `applications/` folders, user's first.
pub(crate) fn data_dirs(env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match env("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(home) => dirs.push(PathBuf::from(home)),
        None => {
            if let Some(home) = env("HOME").filter(|value| !value.is_empty()) {
                dirs.push(PathBuf::from(home).join(".local/share"));
            }
        }
    }
    let system = env("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(
        system
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
    );
    dirs
}

/// The `Name=` of `applications/<app_id>.desktop` in the first directory
/// that has the file (the line without a locale).
pub(crate) fn desktop_name(app_id: &str, dirs: &[PathBuf]) -> Option<String> {
    let file = format!("{app_id}.desktop");
    dirs.iter().find_map(|dir| {
        let text = fs::read_to_string(dir.join("applications").join(&file)).ok()?;
        let mut in_entry = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
            } else if in_entry {
                if let Some(name) = line.strip_prefix("Name=") {
                    return Some(name.trim().to_string());
                }
            }
        }
        None
    })
}

/// The apps of the active units in `units` (name, active state): each app id
/// once, in order, with its display name from `lookup`.
pub(crate) fn apps_of_units(
    units: &[(String, String)],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<AppEntry> {
    let mut apps: Vec<AppEntry> = Vec::new();
    for (name, state) in units {
        if state != "active" {
            continue;
        }
        let Some(unit) = parse_app_unit(name) else {
            continue;
        };
        if apps.iter().any(|app| app.app_id == unit.app_id) {
            continue;
        }
        let display = lookup(&unit.app_id).unwrap_or_else(|| unit.app_id.clone());
        apps.push(AppEntry {
            app_id: unit.app_id,
            name: display,
            icon: None,
            pid: None,
        });
    }
    apps
}

type UnitRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    zbus::zvariant::OwnedObjectPath,
    u32,
    String,
    zbus::zvariant::OwnedObjectPath,
);

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("systemd: {error}"))
}

/// The apps in the user's systemd instance on the session bus.
pub(crate) fn list() -> io::Result<WindowList> {
    let connection = zbus::blocking::Connection::session().map_err(failed)?;
    let manager = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .map_err(failed)?;
    let rows: Vec<UnitRow> = manager.call("ListUnits", &()).map_err(failed)?;
    let units: Vec<(String, String)> = rows.into_iter().map(|row| (row.0, row.3)).collect();
    let dirs = data_dirs(&|name| std::env::var(name).ok());
    Ok(WindowList {
        windows: Vec::new(),
        apps: apps_of_units(&units, &|app_id| desktop_name(app_id, &dirs)),
        list_source: ListSource::SystemdUnits,
        limits: Vec::new(),
    })
}
