//! DSK-M8 checks of the shell's side of the extension (`GnomeSteward`): the
//! control file and the generation, the surfaces that connect, the consent
//! to the take-over of the foreign desktop icons and its return, and
//! `desktop_task_linux_disable_leaves_no_files` (activation, take-over and
//! channel, then `disable` and `remove` leave no file and no setting of se,
//! and the foreign values as they were).
use std::cell::{Cell, RefCell};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::gnome_control::{RuntimeFiles, PROTOCOL};
use super::gnome_steward::{Arrival, GnomeSteward, Wanted};
use super::gnome_test_support::{FakeShell, Store};
use super::integration::LinuxIntegration;
use super::session_facts::DESKTOP_ICON_EXTENSIONS;
use super::task_fixtures;
use crate::desktop::core::config::{ForeignDesktopIcons, SystemItems};
use crate::desktop::core::ledger::{removal_plan, Registration, GNOME_EXTENSION_UUID};
use crate::desktop::core::mechanism::{GnomeVariant, Mechanism};
use crate::desktop::core::protocol::{
    read_frame, write_frame, ShellMessage, StopReason, SurfaceMessage, PROTOCOL_VERSION,
};
use crate::desktop::core::state::{ModuleState, Part, PartSet, State, STATE_SCHEMA};
use crate::desktop::os::control::{
    Control as Transitions, Integration, ShellProcesses, StopReport,
};
use crate::desktop::os::store::DesktopDir;
use crate::desktop::os::Monitor;

fn runtime(root: &Path) -> RuntimeFiles {
    RuntimeFiles::at(root.join("run").join("smart-explorer"))
}

fn hello(monitor: &str, token: Option<&str>) -> SurfaceMessage {
    SurfaceMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: monitor.into(),
        pid: 99,
        token: token.map(str::to_string),
    }
}

struct Harness {
    root: tempfile::TempDir,
    store: Arc<Store>,
    shell: Arc<FakeShell>,
    dir: DesktopDir,
    files: RuntimeFiles,
    steward: GnomeSteward,
}

fn write_state(dir: &DesktopDir, ledger: Vec<crate::desktop::core::ledger::LedgerEntry>) {
    dir.create().unwrap();
    dir.write_state(&ModuleState {
        schema: STATE_SCHEMA,
        state: State::Enabled,
        parts: PartSet::of(&[Part::Surface]),
        changed_at: 1,
        halted: None,
        ledger,
    })
    .unwrap();
}

fn harness(with_ding: bool) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let paths = task_fixtures::paths(root.path());
    let store = Arc::new(Store::new(root.path()));
    let shell = FakeShell::new(&store, "46.0", false);
    if with_ding {
        let uuid = DESKTOP_ICON_EXTENSIONS[0];
        let folder = paths.extensions_dir.join(uuid);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("metadata.json"), "{}").unwrap();
        store.set_list("enabled-extensions", &[uuid, "foreign@example.org"]);
        shell.load(uuid);
    }
    let dir = DesktopDir::at(root.path().join("ad").join("desktop"));
    write_state(&dir, Vec::new());
    let files = runtime(root.path());
    let steward = GnomeSteward::with(
        dir.clone(),
        PathBuf::from("/opt/se/smart_explorer"),
        Some(files.clone()),
        paths,
        store.settings(),
        shell.shared(),
    );
    Harness {
        root,
        store,
        shell,
        dir,
        files,
        steward,
    }
}

fn monitor(key: &str, x: i32) -> Monitor {
    Monitor {
        key: key.into(),
        primary: x == 0,
        rect: Some([x, 0, x + 1920, 1080]),
    }
}

const EXTENSION: Mechanism = Mechanism::GnomeExtension {
    variant: GnomeVariant::A,
};

fn wanted<'a>(
    mechanism: &'a Mechanism,
    surface_on: bool,
    monitors: &'a [Monitor],
    icons: ForeignDesktopIcons,
) -> Wanted<'a> {
    Wanted {
        mechanism,
        surface_on,
        monitors,
        icons,
    }
}

fn arrival(steward: &mut GnomeSteward) -> Option<Arrival> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(found) = steward.arrivals().into_iter().next() {
            return Some(found);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn gnome_steward_publishes_the_control_file_and_takes_surfaces() {
    let mut h = harness(false);
    let two = [monitor("DP-1", 0), monitor("HDMI-1", 1920)];
    let keep = ForeignDesktopIcons::Keep;
    let round = h.steward.sync(&wanted(&EXTENSION, true, &two, keep));
    assert!(round.active && !round.session_changed);
    let first = h.files.read_control().expect("desktop-gnome.json");
    assert_eq!(first.protocol, PROTOCOL);
    assert_eq!(first.exe, "/opt/se/smart_explorer");
    assert_eq!(first.socket, h.files.socket_path().to_string_lossy());
    assert_eq!(first.monitors.len(), 2);
    assert_eq!(
        (first.monitors[1].index, first.monitors[1].key.as_str()),
        (1, "HDMI-1")
    );
    assert_eq!((first.monitors[1].x, first.monitors[1].width), (1920, 1920));
    // The extension and its ledger entries.
    let state = h.dir.read_state().unwrap().unwrap();
    let kinds: Vec<_> = state.ledger.iter().map(|entry| entry.kind.key()).collect();
    assert_eq!(kinds, ["gnome_extension_files", "gnome_extension_enabled"]);
    assert!(h
        .store
        .list("enabled-extensions")
        .contains(&GNOME_EXTENSION_UUID.to_string()));

    // The same monitors: the same generation; other monitors: one more.
    h.steward.sync(&wanted(&EXTENSION, true, &two, keep));
    assert_eq!(h.files.read_control().unwrap().generation, first.generation);
    // A surface connects as monitor 1.
    let token = h.files.read_token().expect("token");
    let mut client = UnixStream::connect(h.files.socket_path()).unwrap();
    write_frame(&mut client, &hello("gnome-1", Some(&token))).unwrap();
    match arrival(&mut h.steward) {
        Some(Arrival::Surface { key, mut end }) => {
            assert_eq!(key, "HDMI-1");
            assert_eq!(end.pid, std::process::id());
            // The shell can write to it and read from it.
            write_frame(
                &mut end.writer,
                &ShellMessage::Stop {
                    reason: StopReason::Restart,
                },
            )
            .unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let message = read_frame::<_, ShellMessage>(&mut client).unwrap();
            assert!(matches!(message, Some(ShellMessage::Stop { .. })));
            write_frame(&mut client, &SurfaceMessage::Alive).unwrap();
            let alive = read_frame::<_, SurfaceMessage>(&mut end.reader).unwrap();
            assert_eq!(alive, Some(SurfaceMessage::Alive));
        }
        _ => panic!("a surface expected"),
    }
    // A monitor that is not in the file is refused.
    let mut stray = UnixStream::connect(h.files.socket_path()).unwrap();
    write_frame(&mut stray, &hello("gnome-7", Some(&token))).unwrap();
    assert!(matches!(
        arrival(&mut h.steward),
        Some(Arrival::Rejected(_))
    ));

    let one = [monitor("DP-1", 0)];
    h.steward.sync(&wanted(&EXTENSION, true, &one, keep));
    let second = h.files.read_control().unwrap();
    assert_eq!(second.generation, first.generation + 1);
    assert_eq!(second.monitors.len(), 1);

    // The surface part off, or another mechanism: the extension has nothing
    // to run, the files stay.
    let round = h.steward.sync(&wanted(&EXTENSION, false, &one, keep));
    assert!(!round.active);
    assert!(!h.files.control_path().exists());
    assert!(task_fixtures::paths(h.root.path())
        .extensions_dir
        .join(GNOME_EXTENSION_UUID)
        .exists());
    // Back on: a higher generation than ever.
    h.steward.sync(&wanted(&EXTENSION, true, &one, keep));
    assert!(h.files.read_control().unwrap().generation > second.generation);
    // The shell's end.
    h.steward.finish();
    assert!(!h.files.control_path().exists());
    assert!(!h.files.token_path().exists());
    assert!(!h.files.socket_path().exists());
}

#[test]
fn gnome_steward_takes_over_icons_only_with_consent_and_gives_them_back() {
    let mut h = harness(true);
    let ding = DESKTOP_ICON_EXTENSIONS[0];
    let two = [monitor("DP-1", 0)];
    let keep = ForeignDesktopIcons::Keep;
    let take = ForeignDesktopIcons::TakeOver;
    let unavailable =
        Mechanism::Unavailable(crate::desktop::core::mechanism::Reason::NoDesktopLayer);

    // `keep` with DING active: the mechanism is unavailable, nothing is called.
    let round = h.steward.sync(&wanted(&unavailable, true, &two, keep));
    assert!(!round.active && !round.session_changed);
    assert!(h.shell.calls().is_empty());
    assert!(h
        .store
        .list("enabled-extensions")
        .contains(&ding.to_string()));

    // The consent: DING off, one ledger entry with its UUID; the facts of the
    // session changed.
    let round = h.steward.sync(&wanted(&EXTENSION, true, &two, take));
    assert!(round.active && round.session_changed);
    assert!(h.shell.calls().contains(&format!("disable {ding}")));
    let state = h.dir.read_state().unwrap().unwrap();
    let taken: Vec<_> = state
        .ledger
        .iter()
        .filter(|entry| entry.kind == Registration::ForeignExtensionDisabled)
        .map(|entry| entry.target.clone())
        .collect();
    assert_eq!(taken, [ding]);
    assert_eq!(
        h.store.list("enabled-extensions"),
        ["foreign@example.org", GNOME_EXTENSION_UUID]
    );
    // Not again with every round.
    let calls = h.shell.calls().len();
    h.steward.sync(&wanted(&EXTENSION, true, &two, take));
    assert_eq!(h.shell.calls().len(), calls);

    // Back to `keep`: DING is switched on again and the entry is gone.
    let round = h.steward.sync(&wanted(&EXTENSION, true, &two, keep));
    assert!(round.session_changed);
    assert!(h.shell.calls().contains(&format!("enable {ding}")));
    assert!(h
        .store
        .list("enabled-extensions")
        .contains(&ding.to_string()));
    let state = h.dir.read_state().unwrap().unwrap();
    assert!(state
        .ledger
        .iter()
        .all(|entry| entry.kind != Registration::ForeignExtensionDisabled));
}

struct Processes {
    running: Cell<bool>,
    started: RefCell<usize>,
}

impl ShellProcesses for Processes {
    fn running(&self, _dir: &DesktopDir) -> bool {
        self.running.get()
    }
    fn start(&self, _gui: &Path) -> Result<(), String> {
        *self.started.borrow_mut() += 1;
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

#[test]
fn desktop_task_linux_disable_leaves_no_files() {
    for remove in [false, true] {
        let mut h = harness(true);
        let root = h.root.path().to_path_buf();
        task_fixtures::plant_foreign(&root);
        let gui = root.join("app").join("smart_explorer");
        let before = task_fixtures::trace_snapshot(&root);
        let enabled_before = h.store.list("enabled-extensions");
        let paths = task_fixtures::paths(&root);
        let mut integration = LinuxIntegration::with(paths, h.store.settings(), h.shell.bus());
        let processes = Processes {
            running: Cell::new(false),
            started: RefCell::new(0),
        };
        let mut available = PartSet::default();
        available.set(Part::Surface, true);
        let mut transitions = Transitions {
            dir: h.dir.clone(),
            integration: &mut integration,
            processes: &processes,
            gui: Ok(gui),
            available,
            unavailable: Vec::new(),
            system_items: SystemItems::initial(None, None),
            version: env!("CARGO_PKG_VERSION").into(),
            now: 1_700_000_000,
        };
        transitions.enable(None).unwrap();
        // The shell's work: the extension, the take-over, the channel.
        let two = [monitor("DP-1", 0)];
        let round = h.steward.sync(&wanted(
            &EXTENSION,
            true,
            &two,
            ForeignDesktopIcons::TakeOver,
        ));
        assert!(round.active && round.session_changed);
        assert!(h.files.control_path().exists() && h.files.token_path().exists());
        let during = task_fixtures::trace_snapshot(&root);
        assert_ne!(before, during, "the test has traces to remove");

        let outcome = if remove {
            transitions.remove().unwrap()
        } else {
            transitions.disable().unwrap()
        };
        assert!(outcome.complete(), "{outcome:?}");
        assert_eq!(
            before,
            task_fixtures::trace_snapshot(&root),
            "remove={remove}: no file of se is left, no foreign file changed"
        );
        assert!(task_fixtures::foreign_present(&root));
        let mut enabled_after = h.store.list("enabled-extensions");
        enabled_after.sort();
        let mut enabled_before = enabled_before;
        enabled_before.sort();
        assert_eq!(
            enabled_after, enabled_before,
            "the list is as it was, DING included (the Shell appends it)"
        );
        assert!(h.store.list("disabled-extensions").is_empty());
        for trace in removal_plan(crate::desktop::core::state::Platform::Linux, &[]) {
            assert!(!integration_is_applied(&root, &h, &trace), "{trace:?}");
        }
        assert_eq!(remove, !h.dir.exists());
    }
}

fn integration_is_applied(
    root: &Path,
    h: &Harness,
    trace: &crate::desktop::core::ledger::Trace,
) -> bool {
    LinuxIntegration::with(
        task_fixtures::paths(root),
        h.store.settings(),
        h.shell.bus(),
    )
    .is_applied(trace)
}
