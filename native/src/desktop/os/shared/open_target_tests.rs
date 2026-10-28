//! DSK-M4: what the surface opens and with which exact arguments of the GUI
//! program; the start parser of V9 reads every one back unchanged.
use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{
    all_screens, cli_beside, device_folder, device_page, device_screen, launch, open_device,
    open_vault, room_screens, OpenRequest,
};
use crate::app::start_request::{self, StartPage, StartRequest};
use crate::desktop::core::config::OpenDevicesWith;
use crate::desktop::core::layout::ItemId;
use crate::desktop::core::tiles::{self, DeviceTile, RoomGroup, TileCode, TileOpen, VaultTile};
use crate::share::PeerOpenTarget;

fn direct(contact_id: &str) -> PeerOpenTarget {
    PeerOpenTarget::Direct {
        contact_id: contact_id.into(),
    }
}

fn member(room_id: &str, device_id: &str) -> PeerOpenTarget {
    PeerOpenTarget::RoomDevice {
        room_id: room_id.into(),
        device_id: device_id.into(),
    }
}

/// A tile as `tiles::project` builds it.
fn tile(target: PeerOpenTarget, code: TileCode) -> DeviceTile {
    let location = format!("{}/", target.endpoint_prefix());
    let (page, device_id) = match &target {
        PeerOpenTarget::Direct { contact_id } => (
            format!("share-device:{contact_id}"),
            Some("geraet-7".into()),
        ),
        PeerOpenTarget::RoomDevice { room_id, device_id } => {
            (format!("share-room:{room_id}"), Some(device_id.clone()))
        }
    };
    let open = if code.opens_files() {
        TileOpen::Location(location.clone())
    } else {
        TileOpen::Page(page)
    };
    DeviceTile {
        key: tiles::device_key(&target),
        target,
        name: "Laptop".into(),
        code,
        status_text: String::new(),
        last_seen: None,
        lan: false,
        online: true,
        write: None,
        location,
        device_id,
        folder_path: None,
        open,
        feed: Default::default(),
    }
}

/// Runs `request` and returns the started program with its arguments, or
/// the opened path.
fn run(request: &OpenRequest, gui: &Path) -> (Vec<(PathBuf, Vec<String>)>, Vec<(PathBuf, bool)>) {
    let spawned = RefCell::new(Vec::new());
    let opened = RefCell::new(Vec::new());
    let spawn = |program: &Path, args: &[String]| -> Result<(), String> {
        spawned
            .borrow_mut()
            .push((program.to_path_buf(), args.to_vec()));
        Ok(())
    };
    let open_path =
        |path: &Path, choose: bool| opened.borrow_mut().push((path.to_path_buf(), choose));
    launch(request, gui, &spawn, &open_path).unwrap();
    (spawned.into_inner(), opened.into_inner())
}

fn parsed(args: &[String]) -> StartRequest {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    start_request::parse(&args).unwrap()
}

fn window_of(request: &OpenRequest, gui: &Path) -> Vec<String> {
    let (spawned, opened) = run(request, gui);
    assert!(opened.is_empty(), "{opened:?}");
    assert_eq!(spawned.len(), 1);
    assert_eq!(spawned[0].0, gui);
    spawned[0].1.clone()
}

#[test]
fn desktop_task_open_device_spawns_gui_location() {
    let gui = PathBuf::from("/opt/se/smart_explorer");
    for target in [direct("c1"), member("raum-1", "d1")] {
        let device = tile(target.clone(), TileCode::ConnectedRelay);
        for with in [OpenDevicesWith::App, OpenDevicesWith::FileManager] {
            // Without a mounted device tree "Dateimanager" opens the app too.
            let request = open_device(&device, with);
            let args = window_of(&request, &gui);
            let locator = format!("{}/", target.endpoint_prefix());
            assert_eq!(args, ["--open-location", locator.as_str()]);
            assert_eq!(
                parsed(&args),
                StartRequest::OpenLocation {
                    locator: locator.clone()
                }
            );
            // F7: the window navigates to exactly this Share location.
            let (back, path) = PeerOpenTarget::from_endpoint(&locator).unwrap();
            assert_eq!(back, target);
            assert!(path.trim_matches('/').is_empty(), "{path}");
        }
    }
    // Every state that may open files opens the location, offline included.
    for code in [TileCode::Unknown, TileCode::Offline, TileCode::Failed] {
        let request = open_device(&tile(direct("c2"), code), OpenDevicesWith::App);
        assert!(matches!(
            request,
            OpenRequest::Window(StartRequest::OpenLocation { .. })
        ));
    }
    // "se Desktop neu starten": `se desktop restart` beside the program, or
    // the settings page when no `se` is installed there.
    let dir = tempfile::tempdir().unwrap();
    let gui = dir.path().join("smart_explorer");
    let settings = window_of(&OpenRequest::Restart, &gui);
    assert_eq!(settings, ["--open-page", "settings-integration"]);
    std::fs::write(cli_beside(&gui), b"").unwrap();
    let (spawned, _) = run(&OpenRequest::Restart, &gui);
    assert_eq!(
        spawned,
        [(
            cli_beside(&gui),
            vec!["desktop".to_string(), "restart".to_string()]
        )]
    );
}

#[test]
fn desktop_task_open_device_file_manager_path() {
    let gui = PathBuf::from("/opt/se/smart_explorer");
    let folder = "/home/nutzer/SmartExplorer/Geräte/Laptop";
    let mut device = tile(direct("c3"), TileCode::Connected);
    device.folder_path = Some(folder.into());
    // "Geräte öffnen mit: Dateimanager" opens the folder of the device tree.
    let request = open_device(&device, OpenDevicesWith::FileManager);
    assert_eq!(request, OpenRequest::Folder(PathBuf::from(folder)));
    let (spawned, opened) = run(&request, &gui);
    assert!(spawned.is_empty());
    assert_eq!(opened, [(PathBuf::from(folder), false)]);
    // "Geräte öffnen mit: App" keeps the se window.
    assert!(matches!(
        open_device(&device, OpenDevicesWith::App),
        OpenRequest::Window(StartRequest::OpenLocation { .. })
    ));
    // The menu entry "Im Explorer/Dateimanager öffnen" exists only with a
    // folder and only where files may be opened.
    assert_eq!(
        device_folder(&device),
        Some(OpenRequest::Folder(folder.into()))
    );
    let mut unmounted = device.clone();
    unmounted.folder_path = None;
    assert_eq!(device_folder(&unmounted), None);
    for code in [TileCode::WaitingForAccess, TileCode::IdentityConflict] {
        let mut closed = tile(direct("c4"), code);
        closed.folder_path = Some(folder.into());
        assert_eq!(device_folder(&closed), None);
    }
}

#[test]
fn desktop_task_identity_conflict_opens_device_page() {
    let gui = PathBuf::from("/opt/se/smart_explorer");
    let mut conflict = tile(direct("c9"), TileCode::IdentityConflict);
    conflict.folder_path = Some("/home/nutzer/SmartExplorer/Geräte/c9".into());
    for with in [OpenDevicesWith::App, OpenDevicesWith::FileManager] {
        let args = window_of(&open_device(&conflict, with), &gui);
        assert_eq!(args, ["--open-page", "share-device:c9"]);
        assert_eq!(
            parsed(&args),
            StartRequest::OpenPage {
                page: StartPage::ShareDevice {
                    contact_id: "c9".into()
                }
            }
        );
    }
    // Never files, never the viewer, after an identity change.
    assert_eq!(device_folder(&conflict), None);
    assert_eq!(device_screen(&conflict, false), None);
    assert_eq!(device_screen(&conflict, true), None);
    // A room member opens its room's page; a missing grant opens the page too.
    let member_conflict = tile(member("raum-2", "d2"), TileCode::IdentityConflict);
    let args = window_of(&open_device(&member_conflict, OpenDevicesWith::App), &gui);
    assert_eq!(args, ["--open-page", "share-room:raum-2"]);
    let waiting = tile(direct("c5"), TileCode::WaitingForAccess);
    let args = window_of(&open_device(&waiting, OpenDevicesWith::App), &gui);
    assert_eq!(args, ["--open-page", "share-device:c5"]);
    // "Geräteseite" of the context menu, for every state.
    let connected = tile(direct("c6"), TileCode::Connected);
    assert_eq!(
        window_of(&device_page(&connected), &gui),
        ["--open-page", "share-device:c6"]
    );
    assert_eq!(
        window_of(&device_page(&member_conflict), &gui),
        ["--open-page", "share-room:raum-2"]
    );
    // An ID the start parser refuses opens the overview of the tab.
    let odd = tile(direct("mit leerzeichen"), TileCode::IdentityConflict);
    assert_eq!(
        window_of(&open_device(&odd, OpenDevicesWith::App), &gui),
        ["--open-page", "share-devices"]
    );
}

#[test]
fn desktop_task_open_vault_location_args() {
    let gui = PathBuf::from("/opt/se/smart_explorer");
    let site = "0123456789abcdef0123456789abcdef";
    for state in ["locked", "unlocked"] {
        let vault = VaultTile {
            site: site.into(),
            label: "Privat".into(),
            state: state.into(),
        };
        let args = window_of(&open_vault(&vault), &gui);
        let locator = format!("vault://{site}/");
        assert_eq!(args, ["--open-location", locator.as_str()]);
        assert_eq!(parsed(&args), StartRequest::OpenLocation { locator });
    }
    // The arrangement keeps only the site ID (S5-V4, S5-V5).
    let item = ItemId::Vault { site: site.into() };
    assert_eq!(item.encode(), format!("vault:{site}"));
}

#[test]
fn desktop_task_screen_view_args_parse_in_s4() {
    let gui = PathBuf::from("/opt/se/smart_explorer");
    for (target, relation, relation_id, device_id) in [
        (direct("c7"), "direct", "c7", "geraet-7"),
        (member("raum-3", "d3"), "room", "raum-3", "d3"),
    ] {
        let device = tile(target, TileCode::Connected);
        for control in [false, true] {
            let request = device_screen(&device, control).unwrap();
            let args = window_of(&request, &gui);
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], "--screen-view");
            let StartRequest::ScreenView { target_json } = parsed(&args) else {
                panic!("kein Betrachterstart: {args:?}");
            };
            // V5 target: exactly these fields; `source: null` is the whole
            // screen (Annahme A28; the S4 parser of SCR-M7 reads this form).
            let value: serde_json::Value = serde_json::from_str(&target_json).unwrap();
            let object = value.as_object().unwrap();
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                ["control", "device_id", "relation", "relation_id", "source"]
            );
            assert_eq!(value["relation"], relation);
            assert_eq!(value["relation_id"], relation_id);
            assert_eq!(value["device_id"], device_id);
            assert!(value["source"].is_null());
            assert_eq!(value["control"], control);
        }
    }
    // Without a device ID there is no viewer target.
    let mut unknown = tile(direct("c8"), TileCode::Connected);
    unknown.device_id = None;
    assert_eq!(device_screen(&unknown, false), None);
    // "Bildschirme des Raums" and "Alle Bildschirme".
    let room = RoomGroup {
        room_id: "raum-4".into(),
        name: "Büro".into(),
        page: "share-room:raum-4".into(),
        members: Vec::new(),
    };
    let args = window_of(&room_screens(&room), &gui);
    assert_eq!(args, ["--screen-grid", "--room", "raum-4"]);
    assert_eq!(
        parsed(&args),
        StartRequest::ScreenGrid {
            room: Some("raum-4".into())
        }
    );
    let args = window_of(&all_screens(), &gui);
    assert_eq!(args, ["--screen-grid"]);
    assert_eq!(parsed(&args), StartRequest::ScreenGrid { room: None });
}
