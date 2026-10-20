//! "Öffnen mit …" and the starters that may not run directly (U34-7): the
//! OpenURI portal `org.freedesktop.portal.OpenURI.OpenFile` with the file
//! descriptor of the opened path and `ask = true` (the portal shows its
//! chooser). The answer is a `Request` path that is not waited for. Without
//! a portal (the call fails) the fallback is `xdg-open <file>`. Everything
//! runs on a thread of its own: the bus call may take as long as zbus waits.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Fd, OwnedObjectPath, Value};

use super::desktop_entry_trust::{
    desktop_kind, launch_decision, launch_direct, Launch, SystemTrust,
};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.OpenURI";

/// `OpenFile(parent_window "", fd, {ask: true})`.
fn portal_open(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let bus = Connection::session().map_err(|error| error.to_string())?;
    let proxy = Proxy::new(&bus, SERVICE, PATH, INTERFACE).map_err(|error| error.to_string())?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("ask", Value::Bool(true));
    proxy
        .call::<_, _, OwnedObjectPath>("OpenFile", &("", Fd::from(&file), options))
        .map(drop)
        .map_err(|error| error.to_string())
}

/// `xdg-open <file>` without any stream of ours, collected on a thread.
fn xdg_open(path: &Path) -> Result<(), String> {
    let mut child = std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("xdg-open: {error}"))?;
    let _ = std::thread::Builder::new()
        .name("xdg-open-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// The portal with its chooser; `xdg-open` when no portal answers.
pub(crate) fn open_with(path: &Path) -> Result<(), String> {
    portal_open(path).or_else(|_| xdg_open(path))
}

fn is_starter(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
        && path.is_file()
}

/// The hook of the surface's `open_path` (desktop files and folders):
/// "Öffnen mit …" and a starter are handled here, on a thread of their own
/// (`true`); everything else stays with `xdg-open` of the app (`false`).
pub(crate) fn open_desktop_path(path: &Path, choose: bool) -> bool {
    let starter = !choose && is_starter(path);
    if !choose && !starter {
        return false;
    }
    let path: PathBuf = path.to_path_buf();
    let work = move || {
        if starter {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            let decision = launch_decision(&path, &desktop_kind(&desktop), &SystemTrust);
            if decision == Launch::Direct && launch_direct(&path) {
                return;
            }
        }
        if let Err(error) = open_with(&path) {
            eprintln!("se Desktop: {}: {error}", path.display());
        }
    };
    std::thread::Builder::new()
        .name("se-desktop-open".into())
        .spawn(work)
        .is_ok()
}
