//! Preserve the selected locator when opening a folder from the surface.
use std::path::{Path, PathBuf};

use crate::app::start_request::StartRequest;
use crate::desktop::core::config::OpenFoldersWith;

use super::open_target::{desktop_file, OpenRequest};

/// Typed application locators are passed unchanged to the shared navigator.
/// A remote locator must never be passed to a native file-manager path API.
pub(crate) fn folder_location(
    locator: &str,
    with: OpenFoldersWith,
) -> Result<OpenRequest, String> {
    let local = crate::connect::local_endpoint_path(locator)?;
    match with {
        OpenFoldersWith::App => Ok(OpenRequest::Window(StartRequest::OpenLocation {
            locator: locator.to_string(),
        })),
        OpenFoldersWith::FileManager => match local {
            Some(path) => Ok(OpenRequest::Folder(PathBuf::from(path))),
            None => Err("Diesen Remote-Ordner in Smart Explorer öffnen; der Systemdateimanager benötigt einen eingehängten lokalen Pfad.".into()),
        },
    }
}

/// `is_dir` comes from the surface inventory, including directory links.
/// Keep the chosen path, rather than canonicalizing a link to its target.
/// A vanished target is reported by the navigator, never replaced by a root.
pub(crate) fn desktop_entry(
    folder: &Path,
    name: &str,
    is_dir: bool,
    choose: bool,
    with: OpenFoldersWith,
) -> Result<OpenRequest, String> {
    if !is_dir || choose {
        return Ok(desktop_file(folder, name, choose));
    }
    let path = folder.join(name);
    if with == OpenFoldersWith::FileManager {
        return Ok(OpenRequest::Folder(path));
    }
    let locator = path.to_str().ok_or_else(|| {
        "Ordnerpfad enthält Zeichen, die der Smart-Explorer-Startauftrag nicht verlustfrei übertragen kann.".to_string()
    })?;
    folder_location(locator, with)
}
