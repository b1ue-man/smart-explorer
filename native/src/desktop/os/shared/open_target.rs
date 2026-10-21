//! Opening from the surface (DSK-M4, C4.5, C4.7): devices and vaults in a new
//! se window (`--open-location`, F7: the window calls `navigate_to_location`),
//! device and room pages (`--open-page`), the S4 viewer (`--screen-view
//! <ziel-json>`, `--screen-grid [--room <id>]`, V5), folders of the S2 device
//! tree and desktop files through the app's `open_local_path`, and "se
//! Desktop neu starten" through `se desktop restart` beside the GUI program.
//!
//! A device whose peer has not granted access yet or whose identity changed
//! opens its page, never its files (C4.5). Windows start detached with null
//! standard streams: the surface's stdin and stdout are its channel.
use std::path::{Path, PathBuf};

use crate::app::start_request::{StartPage, StartRequest};
use crate::desktop::core::config::OpenDevicesWith;
use crate::desktop::core::items::SystemItem;
use crate::desktop::core::screen_target::ScreenTarget;
use crate::desktop::core::tiles::{
    DeviceTile, EmptyAction, RoomGroup, TileCode, TileOpen, VaultTile,
};
use crate::share::PeerOpenTarget;

/// What one handling of the surface starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenRequest {
    /// A window of the GUI program (V9 start kind).
    Window(StartRequest),
    /// A folder of the S2 device tree in Explorer or the file manager.
    Folder(PathBuf),
    /// A desktop file with its standard program; `choose`: "Öffnen mit".
    File { path: PathBuf, choose: bool },
    /// A Windows shell folder (`shell:RecycleBinFolder`, `shell:MyComputerFolder`).
    ShellFolder(&'static str),
    /// "se Desktop neu starten".
    Restart,
}

/// Starts a process: program and arguments (production: detached).
pub(crate) type SpawnFn<'a> = dyn Fn(&Path, &[String]) -> Result<(), String> + 'a;
/// Opens a path with its standard program; `true`: let the user choose.
pub(crate) type OpenPathFn<'a> = dyn Fn(&Path, bool) + 'a;

fn window(request: StartRequest) -> OpenRequest {
    OpenRequest::Window(request)
}

/// A page of the se window; an ID the start parser would refuse opens the
/// overview of its tab instead.
fn page(value: &str) -> OpenRequest {
    let fallback = if value.starts_with("share-room:") {
        StartPage::ShareRooms
    } else {
        StartPage::ShareDevices
    };
    let page = StartPage::parse(value).unwrap_or(fallback);
    window(StartRequest::OpenPage { page })
}

/// Double click on a device (C4.7): its files in a se window, or with "Geräte
/// öffnen mit: Dateimanager" its folder of the mounted device tree; its page
/// while access is missing or its identity changed (C4.5).
pub(crate) fn open_device(tile: &DeviceTile, with: OpenDevicesWith) -> OpenRequest {
    match &tile.open {
        TileOpen::Page(value) => page(value),
        TileOpen::Location(locator) => match (with, device_folder(tile)) {
            (OpenDevicesWith::FileManager, Some(folder)) => folder,
            _ => window(StartRequest::OpenLocation {
                locator: locator.clone(),
            }),
        },
    }
}

/// "Im Explorer öffnen" / "Im Dateimanager öffnen": only while the device
/// tree is mounted and the device's files may be opened.
pub(crate) fn device_folder(tile: &DeviceTile) -> Option<OpenRequest> {
    if !tile.code.opens_files() {
        return None;
    }
    let folder = tile
        .folder_path
        .as_deref()
        .filter(|folder| !folder.is_empty())?;
    Some(OpenRequest::Folder(PathBuf::from(folder)))
}

/// "Geräteseite": the page of the Direct contact, for a room member the page
/// of its room (S1 shows the access section there).
pub(crate) fn device_page(tile: &DeviceTile) -> OpenRequest {
    match &tile.target {
        PeerOpenTarget::Direct { contact_id } => page(&format!("share-device:{contact_id}")),
        PeerOpenTarget::RoomDevice { room_id, .. } => page(&format!("share-room:{room_id}")),
    }
}

/// "Bildschirm ansehen" (`control = false`) and "Bildschirm steuern": the S4
/// viewer decides rights, consent and support itself. None without a device
/// ID and after an identity change.
pub(crate) fn device_screen(tile: &DeviceTile, control: bool) -> Option<OpenRequest> {
    if tile.code == TileCode::IdentityConflict {
        return None;
    }
    let target_json = ScreenTarget::for_tile(tile, control)?.to_arg().ok()?;
    Some(window(StartRequest::ScreenView { target_json }))
}

/// "Bildschirme des Raums": `--screen-grid --room <local room profile ID>`.
pub(crate) fn room_screens(room: &RoomGroup) -> OpenRequest {
    window(StartRequest::ScreenGrid {
        room: Some(room.room_id.clone()),
    })
}

/// "Raumseite".
pub(crate) fn room_page(room: &RoomGroup) -> OpenRequest {
    page(&room.page)
}

/// The tile "Alle Bildschirme".
pub(crate) fn all_screens() -> OpenRequest {
    window(StartRequest::ScreenGrid { room: None })
}

/// A vault in a se window at its root (S5-V2, S5-V4); locked, the window
/// shows its unlock dialog (Annahme A27). se Desktop has no password field.
pub(crate) fn open_vault(vault: &VaultTile) -> OpenRequest {
    window(StartRequest::OpenLocation {
        locator: vault.location(),
    })
}

/// "Gerät hinzufügen" and "Raum beitreten" (C4.6).
pub(crate) fn empty_action(action: EmptyAction) -> OpenRequest {
    page(action.page())
}

/// "Geräte & Freigaben öffnen" when the Share profiles cannot be read (C4.6).
pub(crate) fn share_page() -> OpenRequest {
    window(StartRequest::OpenPage {
        page: StartPage::ShareDevices,
    })
}

/// "se Desktop-Einstellungen …" and "Hintergrund ändern …".
pub(crate) fn settings_page() -> OpenRequest {
    window(StartRequest::OpenPage {
        page: StartPage::SettingsIntegration,
    })
}

/// "Papierkorb" and "Dieser PC" (Windows).
pub(crate) fn system_item(item: SystemItem) -> OpenRequest {
    OpenRequest::ShellFolder(item.shell_folder())
}

/// A desktop file in `folder`; `choose`: "Öffnen mit …".
pub(crate) fn desktop_file(folder: &Path, name: &str, choose: bool) -> OpenRequest {
    OpenRequest::File {
        path: folder.join(name),
        choose,
    }
}

/// The arguments of the GUI program for a window; `start_request::parse`
/// reads them back unchanged (V9).
pub(crate) fn window_args(request: &StartRequest) -> Vec<String> {
    request
        .to_args()
        .into_iter()
        .filter_map(|arg| arg.into_string().ok())
        .collect()
}

/// `se` beside the GUI program, where both installers and development builds
/// put it.
pub(crate) fn cli_beside(gui: &Path) -> PathBuf {
    gui.with_file_name(format!("se{}", std::env::consts::EXE_SUFFIX))
}

/// Starts `request`: windows as processes of `gui`, the restart through `se`
/// (without `se` the settings page, where "Neu starten" waits), folders,
/// files and shell folders through `open_path`.
pub(crate) fn launch(
    request: &OpenRequest,
    gui: &Path,
    spawn: &SpawnFn<'_>,
    open_path: &OpenPathFn<'_>,
) -> Result<(), String> {
    match request {
        OpenRequest::Window(start) => spawn(gui, &window_args(start)),
        OpenRequest::Folder(folder) => {
            open_path(folder, false);
            Ok(())
        }
        OpenRequest::File { path, choose } => {
            open_path(path, *choose);
            Ok(())
        }
        OpenRequest::ShellFolder(target) => {
            open_path(Path::new(target), false);
            Ok(())
        }
        OpenRequest::Restart => {
            let cli = cli_beside(gui);
            if cli.is_file() {
                spawn(&cli, &["desktop".to_string(), "restart".to_string()])
            } else {
                let settings = StartRequest::OpenPage {
                    page: StartPage::SettingsIntegration,
                };
                spawn(gui, &window_args(&settings))
            }
        }
    }
}

/// The production start: detached from the surface, null standard streams,
/// reaped in the background.
pub(crate) fn spawn_detached(program: &Path, args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = super::platform::process::spawn_detached(program, &args)
        .map_err(|error| format!("{}: {error}", program.display()))?;
    super::launch::reap(child);
    Ok(())
}

#[cfg(test)]
#[path = "open_target_tests.rs"]
mod tests;
