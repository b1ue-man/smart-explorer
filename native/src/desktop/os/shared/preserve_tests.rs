//! DSK-M3 preservation cases (strang-desktop.md F6, F7, F10, F11): release
//! assets and feed files, Share navigation, the Explorer context-menu entry
//! and the sync daemon's login start stay as they were.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::control::{Control, ShellProcesses, StopReport};
use super::platform::{self, task_fixtures, PlatformIntegration};
use super::store::DesktopDir;
use crate::app::start_request::{parse, StartRequest};
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::ledger::removal_plan;
use crate::desktop::core::state::{Part, PartSet};
use crate::share::PeerOpenTarget;

/// The Windows traces (Run value, navigation pane keys, jump list, context
/// menu) live in the one HKCU of the account, so two transitions at once
/// see each other's registrations (run 3: `disable` left residue). Tests of
/// this file that change them run one after the other; the Linux fixtures
/// have a folder per test and are not affected.
static ACCOUNT_TRACES: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    ACCOUNT_TRACES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Values between `marker"` and the next `"` in `text`.
fn quoted_after(text: &str, marker: &str) -> Vec<String> {
    text.match_indices(marker)
        .filter_map(|(at, _)| {
            let rest = &text[at + marker.len()..];
            rest.find('"').map(|end| rest[..end].to_string())
        })
        .collect()
}

/// F6: 20 release assets, 16 feed files, the payload hashes of both
/// updaters; se Desktop ships inside the GUI program (C5.1 L1).
#[test]
fn desktop_task_release_assets_unchanged() {
    let publication = include_str!("../../../../release-publication.ps1");
    // The leading space excludes `InstallerPublishedName = "..."` (the map
    // result field, not an asset entry).
    let assets = quoted_after(publication, " PublishedName = \"");
    assert_eq!(
        assets,
        [
            "Smart.Explorer.Setup.$Version.exe",
            "smart_explorer.exe",
            "smart_explorer_updater.exe",
            "se.exe",
            "smart_explorer.exe.sha256",
            "smart_explorer_updater.exe.sha256",
            "se.exe.sha256",
            "smart_explorer",
            "smart_explorer_updater",
            "se",
            "smart_explorer.sha256",
            "smart_explorer_updater.sha256",
            "se.sha256",
            "smart-explorer-android.apk",
            "smart-explorer-android.apk.sha256",
            "install-linux.sh",
            "smart_explorer_command.dll",
            "se-share-server.exe",
            "se-share-server-linux",
            "version.txt",
        ]
    );
    assert!(publication.contains("$items.Count -ne 20"));

    let feed = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("release-native")
        .join("update-feed");
    let mut files: Vec<String> = std::fs::read_dir(&feed)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "se",
            "se.exe",
            "se.exe.sha256",
            "se.sha256",
            "smart-explorer-android.apk",
            "smart-explorer-android.apk.sha256",
            "smart_explorer",
            "smart_explorer.exe",
            "smart_explorer.exe.sha256",
            "smart_explorer.sha256",
            "smart_explorer_updater",
            "smart_explorer_updater.exe",
            "smart_explorer_updater.exe.sha256",
            "smart_explorer_updater.sha256",
            "version.txt",
            "windows-build.manifest",
        ]
    );

    let windows = include_str!("../../../updater/os/windows.rs");
    let linux = include_str!("../../../updater/os/linux_os.rs");
    assert_eq!(
        quoted_after(windows, "hash_name: \""),
        [
            "smart_explorer.exe.sha256",
            "smart_explorer_updater.exe.sha256",
            "se.exe.sha256"
        ]
    );
    assert_eq!(
        quoted_after(linux, "hash_name: \""),
        [
            "smart_explorer.sha256",
            "smart_explorer_updater.sha256",
            "se.sha256"
        ]
    );
}

/// F7: `share://direct/<id>/…` and `share://room/<raum>/<gerät>/…` keep
/// their round trip; `--open-location` carries a tile's location unchanged.
#[test]
fn desktop_task_share_navigation_unchanged() {
    let direct = PeerOpenTarget::Direct {
        contact_id: "c1".into(),
    };
    let room = PeerOpenTarget::RoomDevice {
        room_id: "r1".into(),
        device_id: "d1".into(),
    };
    let cases = [
        ("share://direct/c1/", &direct, "/"),
        (
            "share://direct/c1/Dokumente/a.txt",
            &direct,
            "/Dokumente/a.txt",
        ),
        ("share://room/r1/d1/", &room, "/"),
        ("share://room/r1/d1/Fotos", &room, "/Fotos"),
    ];
    for (endpoint, target, path) in cases {
        let (parsed, parsed_path) = PeerOpenTarget::from_endpoint(endpoint).unwrap();
        assert_eq!(&parsed, target, "{endpoint}");
        assert_eq!(parsed_path, path, "{endpoint}");
        assert!(endpoint.starts_with(&target.endpoint_prefix()));
    }
    for target in [&direct, &room] {
        let location = format!("{}/", target.endpoint_prefix());
        let (parsed, path) = PeerOpenTarget::from_endpoint(&location).unwrap();
        assert_eq!((&parsed, path.as_str()), (target, "/"));
        let request = StartRequest::OpenLocation {
            locator: location.clone(),
        };
        let args: Vec<OsString> = request.to_args();
        assert_eq!(parse(&args), Ok(request));
    }
}

struct NoShell;

impl ShellProcesses for NoShell {
    fn running(&self, _dir: &DesktopDir) -> bool {
        false
    }

    fn start(&self, _gui: &Path) -> Result<(), String> {
        Ok(())
    }

    fn await_stop(&self, _dir: &DesktopDir, _gui: Option<&Path>) -> StopReport {
        StopReport {
            stopped: true,
            ..StopReport::default()
        }
    }
}

/// enable → disable → enable → remove → the uninstaller's removal.
fn run_every_transition(root: &Path, gui: &Path) {
    let mut integration = PlatformIntegration::for_task(root);
    for trace in removal_plan(platform::PLATFORM, &[]) {
        assert!(!super::control::Integration::is_applied(
            &integration,
            &trace
        ));
    }
    let mut available = PartSet::default();
    for part in Part::ALL {
        available.set(part, part.exists_on(platform::PLATFORM));
    }
    let mut control = Control {
        dir: DesktopDir::at(root.join("ad").join("desktop")),
        integration: &mut integration,
        processes: &NoShell,
        gui: Ok(gui.to_path_buf()),
        available,
        unavailable: Vec::new(),
        system_items: SystemItems::initial(None, None),
        version: env!("CARGO_PKG_VERSION").into(),
        now: 1_700_000_000,
    };
    assert!(control.enable(None).unwrap().complete());
    assert!(control.disable().unwrap().complete());
    assert!(control.enable(None).unwrap().complete());
    assert!(control.remove().unwrap().complete());
    assert!(control.remove_system_traces().complete());
}

/// F10: the Explorer entry "In Smart Explorer öffnen" (Windows; Linux has
/// none) survives every transition and the uninstaller's removal.
#[test]
fn desktop_task_context_menu_unchanged() {
    let _serial = one_at_a_time();
    let Some(before) = task_fixtures::context_menu_probe() else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let gui: PathBuf = root.path().join("Smart Explorer.exe");
    // An entry the account already has stays exactly as it is.
    if !before {
        task_fixtures::set_context_menu(true, &gui);
    }
    run_every_transition(root.path(), &gui);
    let after = task_fixtures::context_menu_probe();
    if !before {
        task_fixtures::set_context_menu(false, &gui);
    }
    assert_eq!(after, Some(true));
}

/// F11: the sync daemon's login start (`SmartExplorerSync`,
/// `smart-explorer-sync-daemon.desktop`) stays byte-identical.
#[test]
fn desktop_task_sync_autostart_untouched() {
    let _serial = one_at_a_time();
    let root = tempfile::tempdir().unwrap();
    let gui = root.path().join("app").join("smart_explorer");
    task_fixtures::plant_foreign(root.path());
    let before = task_fixtures::sync_autostart(root.path());
    run_every_transition(root.path(), &gui);
    let after = task_fixtures::sync_autostart(root.path());
    task_fixtures::remove_foreign(root.path());
    assert_eq!(after, before);
}
