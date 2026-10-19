//! „Im Dateimanager öffnen“ (DRV-M5, contract V4): shows folders in the
//! file manager of the session.
//!
//! First `org.freedesktop.FileManager1.ShowFolders` on the session bus (the
//! file managers of GNOME, KDE, Xfce and Cinnamon own that name); when the
//! bus, the service or the call fails, `xdg-open <first folder>`. The call
//! waits at most as long as zbus does (25 s), so interfaces call it on a
//! background thread.
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use zbus::blocking::{Connection, Proxy};

use super::bookmarks::uri_of;

const SERVICE: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";
const INTERFACE: &str = "org.freedesktop.FileManager1";

/// How a folder was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenMethod {
    FileManager1,
    XdgOpen,
}

impl OpenMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileManager1 => "filemanager1",
            Self::XdgOpen => "xdg-open",
        }
    }
}

/// Shows `folders` in the file manager. An empty list is `InvalidInput`.
pub(crate) fn show_folders(folders: &[PathBuf]) -> io::Result<OpenMethod> {
    let Some(first) = folders.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "kein Ordner zum Öffnen",
        ));
    };
    let uris: Vec<String> = folders.iter().map(|folder| uri_of(folder)).collect();
    if show_through_bus(&uris).is_ok() {
        return Ok(OpenMethod::FileManager1);
    }
    open_with_xdg(first)?;
    Ok(OpenMethod::XdgOpen)
}

fn show_through_bus(uris: &[String]) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = Proxy::new(&connection, SERVICE, PATH, INTERFACE)?;
    proxy.call::<_, _, ()>("ShowFolders", &(uris.to_vec(), ""))
}

/// The child is collected on a thread, so no zombie stays behind. The same
/// pattern as `app/os/linux_os.rs`; the mount module may not use `app`.
fn open_with_xdg(folder: &std::path::Path) -> io::Result<()> {
    let mut child = Command::new("xdg-open")
        .arg(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let _ = std::thread::Builder::new()
        .name("xdg-open-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}
