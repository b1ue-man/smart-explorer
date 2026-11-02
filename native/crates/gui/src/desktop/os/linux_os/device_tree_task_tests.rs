//! Signal `desktop_task_disable_keeps_device_tree` (F14 of strand S5, DRV-M5
//! and DRV-M6 of strand S2): disabling or removing se Desktop changes
//! nothing of the device tree of the drives. se Desktop owns its login
//! entry, its extension and its runtime files; the places of the file
//! managers (GTK bookmarks, KDE places), the login entry of the drive
//! restore `smart-explorer-drives.desktop` and the switch "Verbundene Geräte
//! als Ordner" belong to S2 and stay byte for byte as they are.
//!
//! Everything lives below a temporary root (no environment variable is
//! read or changed, so the check is safe wherever it runs; the stage still
//! gives it the fixture `home`). The places are written by the real writer
//! of the drives. The login entry and the two files of the switch are
//! planted in the documented shape: the check compares bytes, so their
//! content does not matter, only that nothing touches them.
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use super::integration::LinuxIntegration;
use super::task_fixtures;
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::ledger::{LedgerEntry, Registration};
use crate::desktop::core::state::{ModuleState, Part, PartSet, State, STATE_SCHEMA};
use crate::desktop::os::control::{
    AutostartState, Control, Integration, ShellProcesses, StopReport,
};
use crate::desktop::os::store::DesktopDir;
use crate::mount::os::linux_os::bookmarks::{add_in, uri_of, BookmarkEntry, Targets};

/// The login entry of the drive restore (S2): the program with
/// `--sync-daemon --drive-restore`.
const DRIVES_ENTRY_NAME: &str = "smart-explorer-drives.desktop";
const DRIVES_ENTRY: &str = concat!(
    "[Desktop Entry]\nType=Application\nName=Smart Explorer Laufwerke\n",
    "Exec=\"/opt/se/smart_explorer\" --sync-daemon --drive-restore\n",
    "X-GNOME-Autostart-enabled=true\n"
);

/// The KDE places of a user before any drive is mounted.
const FOREIGN_PLACES: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<!DOCTYPE xbel>\n",
    "<xbel xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" ",
    "xmlns:kdepriv=\"http://www.kde.org/kdepriv\" ",
    "xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    " <bookmark href=\"file:///home/u\">\n",
    "  <title>Home &amp; Co</title>\n",
    "  <info>\n",
    "   <metadata owner=\"http://www.kde.org\">\n",
    "    <ID>1234</ID>\n",
    "   </metadata>\n",
    "  </info>\n",
    " </bookmark>\n",
    "</xbel>\n"
);

struct World {
    // GLib's display rule rejects any `/.` in a path, so the temporary root
    // must not start with a dot.
    root: tempfile::TempDir,
    targets: Targets,
}

impl World {
    fn new() -> World {
        let root = tempfile::Builder::new()
            .prefix("se-desktop-tree-")
            .tempdir()
            .expect("temporary folder");
        let targets = Targets::resolve(
            Some(root.path().join("config").into_os_string()),
            Some(root.path().join("data").into_os_string()),
            Some(root.path().join("home")),
        )
        .expect("targets of the places");
        World { root, targets }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn gui(&self) -> PathBuf {
        self.path("app/smart_explorer")
    }

    fn desktop_dir(&self) -> DesktopDir {
        DesktopDir::at(self.path("ad/desktop"))
    }

    /// The device tree of the drives, as `se drive devices on` and a mounted
    /// drive leave it: the places, the login entry of the restore and the
    /// two files of the switch.
    fn plant_device_tree(&self) {
        // Other programs' lines come first; the writer adds its own.
        write(&self.targets.gtk, "file:///home/u/Dokumente Dokumente\n");
        write(&self.targets.kde, FOREIGN_PLACES);
        fs::create_dir_all(self.path("mnt")).expect("mount parent");
        fs::create_dir_all(self.path("home/SmartExplorer")).expect("home");
        // The tree on a folder outside home is in both files; a drive below
        // home is shown by GLib itself and is only in the KDE places.
        let drives = [
            ("se-devices", self.path("mnt/Geräte"), "Geräte"),
            ("prod-drive", self.path("home/SmartExplorer/prod"), "prod"),
        ];
        for (id, folder, label) in drives {
            let entry = BookmarkEntry {
                id: id.into(),
                folder,
                label: label.into(),
            };
            add_in(&self.targets, &entry).expect("places of a drive");
        }
        write(
            &self.path("config/autostart").join(DRIVES_ENTRY_NAME),
            DRIVES_ENTRY,
        );
        write(&self.path("ad/mounts/devices.json"), "{\"enabled\":true}\n");
        write(
            &self.path("ad/mounts/registry-2.json"),
            "{\"version\":1,\"mounts\":[]}\n",
        );
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    fs::write(path, text).expect("file");
}

/// What the device tree owns: its files with their bytes (`None`: absent)
/// and the names in the folders around them, which se Desktop never writes.
#[derive(Debug, PartialEq, Eq)]
struct TreeFiles {
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
    folders: Vec<(PathBuf, Vec<String>)>,
}

fn tree_files(world: &World) -> TreeFiles {
    let files = [
        world.targets.gtk.clone(),
        world.targets.kde.clone(),
        world.path("config/autostart").join(DRIVES_ENTRY_NAME),
        world.path("ad/mounts/devices.json"),
        world.path("ad/mounts/registry-2.json"),
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).ok();
        (path, bytes)
    })
    .collect();
    let folders = [
        world.path("config/gtk-3.0"),
        world.path("data"),
        world.path("ad/mounts"),
    ]
    .into_iter()
    .map(|path| {
        let names = names_in(&path);
        (path, names)
    })
    .collect();
    TreeFiles { files, folders }
}

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The tree is really there, so that "unchanged" says something.
fn assert_planted(world: &World, tree: &TreeFiles) {
    for (path, bytes) in &tree.files {
        assert!(bytes.is_some(), "{} was not planted", path.display());
    }
    let gtk = fs::read_to_string(&world.targets.gtk).expect("GTK bookmarks");
    let kde = fs::read_to_string(&world.targets.kde).expect("KDE places");
    assert!(gtk.contains(&uri_of(&world.path("mnt/Geräte"))), "{gtk}");
    assert!(kde.contains("urn:smart-explorer:places"), "{kde}");
    assert!(kde.contains("<id>se-devices</id>"), "{kde}");
    assert!(kde.contains("Home &amp; Co"), "{kde}");
    let decision = tree
        .folders
        .iter()
        .find(|(folder, _)| folder.ends_with("ad/mounts"))
        .map(|(_, names)| names.clone())
        .unwrap_or_default();
    assert_eq!(decision, ["devices.json", "registry-2.json"]);
}

/// The shell of se Desktop as the transitions see it: started and stopped,
/// never a process.
#[derive(Default)]
struct Shell {
    running: Cell<bool>,
}

impl ShellProcesses for Shell {
    fn running(&self, _dir: &DesktopDir) -> bool {
        self.running.get()
    }

    fn start(&self, _gui: &Path) -> Result<(), String> {
        self.running.set(true);
        Ok(())
    }

    fn await_stop(&self, _dir: &DesktopDir, _gui: Option<&Path>) -> StopReport {
        self.running.set(false);
        StopReport {
            stopped: true,
            ..StopReport::default()
        }
    }
}

fn control<'a>(
    world: &World,
    integration: &'a mut LinuxIntegration,
    shell: &'a Shell,
) -> Control<'a> {
    let mut available = PartSet::default();
    available.set(Part::Surface, true);
    Control {
        dir: world.desktop_dir(),
        integration,
        processes: shell,
        gui: Ok(world.gui()),
        available,
        unavailable: Vec::new(),
        system_items: SystemItems::initial(None, None),
        version: env!("CARGO_PKG_VERSION").into(),
        now: 1_700_000_000,
    }
}

/// `state.json` of an enabled module with the given ledger.
fn write_state(world: &World, ledger: Vec<LedgerEntry>) {
    let dir = world.desktop_dir();
    dir.create().expect("folder of se Desktop");
    dir.write_state(&ModuleState {
        schema: STATE_SCHEMA,
        state: State::Enabled,
        parts: PartSet::of(&[Part::Surface]),
        changed_at: 1,
        halted: None,
        ledger,
    })
    .expect("state");
}

#[test]
fn desktop_task_disable_keeps_device_tree() {
    let world = World::new();
    world.plant_device_tree();
    task_fixtures::plant_foreign(world.root.path());
    let tree = tree_files(&world);
    assert_planted(&world, &tree);
    let traces = task_fixtures::trace_snapshot(world.root.path());
    let mut integration = LinuxIntegration::for_task(world.root.path());
    let shell = Shell::default();

    // Enabling, then disabling; enabling again, then removing: the login
    // entry of se Desktop comes and goes, the tree stays as it is.
    for remove in [false, true] {
        let outcome = control(&world, &mut integration, &shell)
            .enable(None)
            .unwrap();
        assert!(outcome.complete() && outcome.started, "{outcome:?}");
        assert_eq!(integration.autostart(), AutostartState::On);
        assert_eq!(tree_files(&world), tree, "after enable");
        let outcome = if remove {
            control(&world, &mut integration, &shell).remove()
        } else {
            control(&world, &mut integration, &shell).disable()
        }
        .unwrap();
        assert!(outcome.complete(), "{outcome:?}");
        assert!(!shell.running.get());
        assert_eq!(integration.autostart(), AutostartState::Off);
        assert_eq!(world.desktop_dir().exists(), !remove);
        assert_eq!(tree_files(&world), tree, "remove={remove}");
        assert_eq!(
            task_fixtures::trace_snapshot(world.root.path()),
            traces,
            "remove={remove}: se Desktop left no trace and took none of S2"
        );
        assert!(task_fixtures::foreign_present(world.root.path()));
    }

    // The uninstaller removes every system trace of se Desktop again.
    let outcome = control(&world, &mut integration, &shell)
        .enable(None)
        .unwrap();
    assert!(outcome.complete(), "{outcome:?}");
    let outcome = control(&world, &mut integration, &shell).remove_system_traces();
    assert!(outcome.complete(), "{outcome:?}");
    assert_eq!(tree_files(&world), tree, "after the uninstaller");
    assert_eq!(task_fixtures::trace_snapshot(world.root.path()), traces);

    // A ledger edited to name the login entry of the drive restore is
    // refused and never followed; the folder of se Desktop stays for a retry.
    write_state(
        &world,
        vec![LedgerEntry {
            kind: Registration::AutostartFile,
            target: DRIVES_ENTRY_NAME.into(),
            applied_at: 1,
        }],
    );
    let outcome = control(&world, &mut integration, &shell).remove().unwrap();
    assert!(!outcome.errors.is_empty(), "{outcome:?}");
    assert!(world.desktop_dir().exists());
    assert_eq!(tree_files(&world), tree, "after a ledger naming S2");
    assert_eq!(task_fixtures::trace_snapshot(world.root.path()), traces);
    assert!(task_fixtures::foreign_present(world.root.path()));
}
