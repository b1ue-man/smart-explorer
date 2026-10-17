//! DSK-M3 transitions with the platform's real registrations and fake shell
//! processes: Linux works in folders below a temporary root; Windows works on
//! the runner's HKCU and leaves it as it found it. A Windows account where
//! se Desktop is in use is refused instead of being changed.
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;

use super::control::{AutostartState, Control, Integration, ShellProcesses, StopReport};
use super::platform::{self, task_fixtures, PlatformIntegration};
use super::store::DesktopDir;
use super::{launch, status};
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::ledger::{removal_plan, LedgerEntry};
use crate::desktop::core::state::{
    Halt, HaltReason, ModuleState, Part, PartSet, State, STATE_SCHEMA,
};

#[derive(Default)]
struct FakeShell {
    running: Cell<bool>,
    started: RefCell<Vec<PathBuf>>,
    stops: Cell<usize>,
}

impl ShellProcesses for FakeShell {
    fn running(&self, _dir: &DesktopDir) -> bool {
        self.running.get()
    }

    fn start(&self, gui: &Path) -> Result<(), String> {
        self.started.borrow_mut().push(gui.to_path_buf());
        self.running.set(true);
        Ok(())
    }

    fn await_stop(&self, _dir: &DesktopDir, _gui: Option<&Path>) -> StopReport {
        self.stops.set(self.stops.get() + 1);
        self.running.set(false);
        StopReport {
            stopped: true,
            ..StopReport::default()
        }
    }
}

struct Fixture {
    root: tempfile::TempDir,
    gui: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let gui = root.path().join("app").join("smart_explorer");
        Fixture { root, gui }
    }

    fn root(&self) -> &Path {
        self.root.path()
    }

    fn dir(&self) -> DesktopDir {
        DesktopDir::at(self.root().join("ad").join("desktop"))
    }

    fn integration(&self) -> PlatformIntegration {
        let integration = PlatformIntegration::for_task(self.root());
        // Never change an account where se Desktop is really in use.
        for trace in removal_plan(platform::PLATFORM, &[]) {
            assert!(
                !integration.is_applied(&trace),
                "precondition: {trace:?} exists before the test"
            );
        }
        integration
    }

    fn control<'a>(
        &self,
        integration: &'a mut dyn Integration,
        shell: &'a dyn ShellProcesses,
    ) -> Control<'a> {
        let mut available = PartSet::default();
        for part in Part::ALL {
            available.set(part, part.exists_on(platform::PLATFORM));
        }
        Control {
            dir: self.dir(),
            integration,
            processes: shell,
            gui: Ok(self.gui.clone()),
            available,
            unavailable: Vec::new(),
            system_items: SystemItems::initial(None, None),
            version: env!("CARGO_PKG_VERSION").into(),
            now: 1_700_000_000,
        }
    }

    fn write_state(&self, halted: Option<&str>, ledger: Vec<LedgerEntry>) {
        let dir = self.dir();
        dir.create().unwrap();
        dir.write_state(&ModuleState {
            schema: STATE_SCHEMA,
            state: State::Enabled,
            parts: PartSet::of(&[Part::Surface]),
            changed_at: 1,
            halted: halted.map(|version| Halt {
                reason: HaltReason::CrashLoop,
                version: version.into(),
                at: 5,
            }),
            ledger,
        })
        .unwrap();
    }
}

fn autostart_present(integration: &PlatformIntegration) -> bool {
    removal_plan(platform::PLATFORM, &[])
        .iter()
        .filter(|trace| trace.kind == integration.autostart_registration())
        .any(|trace| integration.is_applied(trace))
}

#[test]
fn desktop_task_enable_disable_leaves_no_trace() {
    for remove in [false, true] {
        let fixture = Fixture::new();
        let mut integration = fixture.integration();
        let before = task_fixtures::trace_snapshot(fixture.root());
        let shell = FakeShell::default();
        let outcome = fixture
            .control(&mut integration, &shell)
            .enable(None)
            .unwrap();
        assert!(outcome.complete(), "{outcome:?}");
        assert!(outcome.started);
        assert_eq!(shell.started.borrow().as_slice(), [fixture.gui.clone()]);
        let module = fixture.dir().read_state().unwrap().unwrap();
        assert_eq!(module.state, State::Enabled);
        assert!(module.parts.surface);
        assert!(module
            .ledger
            .iter()
            .any(|entry| entry.kind == integration.autostart_registration()));
        assert!(autostart_present(&integration));
        assert_eq!(integration.autostart(), AutostartState::On);
        assert_ne!(task_fixtures::trace_snapshot(fixture.root()), before);
        assert!(status::gather(&fixture.dir(), &integration)
            .residue
            .is_empty());

        let mut control = fixture.control(&mut integration, &shell);
        let outcome = if remove {
            control.remove()
        } else {
            control.disable()
        }
        .unwrap();
        assert!(outcome.complete(), "{outcome:?}");
        assert_eq!(shell.stops.get(), 1);
        assert!(!shell.running.get());
        assert_eq!(task_fixtures::trace_snapshot(fixture.root()), before);
        let report = status::gather(&fixture.dir(), &integration);
        assert!(report.residue.is_empty(), "{:?}", report.residue);
        assert_eq!(report.json()["residue"], json!([]));
        if remove {
            assert!(!fixture.dir().root().exists());
            assert_eq!(report.state, State::Absent);
        } else {
            assert_eq!(report.state, State::Disabled);
            // Disabling keeps configuration and arrangement (C5.7).
            assert!(fixture.dir().config_path().is_file());
        }
    }
}

#[test]
fn desktop_task_remove_without_ledger() {
    for with_state in [false, true] {
        let fixture = Fixture::new();
        let mut integration = fixture.integration();
        task_fixtures::plant_fixed_traces(fixture.root(), &fixture.gui);
        if with_state {
            fixture.write_state(None, Vec::new());
        }
        let planted = removal_plan(platform::PLATFORM, &[])
            .into_iter()
            .filter(|trace| integration.is_applied(trace))
            .count();
        assert!(planted >= 2, "{planted} fixed traces planted");
        let shell = FakeShell::default();
        let outcome = fixture.control(&mut integration, &shell).remove().unwrap();
        assert!(outcome.complete(), "{outcome:?}");
        for trace in removal_plan(platform::PLATFORM, &[]) {
            assert!(!integration.is_applied(&trace), "{trace:?} left");
        }
        assert!(integration.runtime_files().is_empty());
        assert!(!fixture.dir().root().exists());
        assert!(status::gather(&fixture.dir(), &integration)
            .residue
            .is_empty());
    }
}

#[test]
fn desktop_task_foreign_values_untouched() {
    let fixture = Fixture::new();
    let mut integration = fixture.integration();
    task_fixtures::plant_foreign(fixture.root());
    let shell = FakeShell::default();
    {
        let mut control = fixture.control(&mut integration, &shell);
        assert!(control.enable(None).unwrap().complete());
        assert!(control.disable().unwrap().complete());
        assert!(control.enable(None).unwrap().complete());
        assert!(control.remove().unwrap().complete());
        assert!(control.remove_system_traces().complete());
    }
    assert!(task_fixtures::foreign_present(fixture.root()));

    // A ledger edited to name foreign targets is refused, never followed.
    let ledger: Vec<LedgerEntry> = removal_plan(platform::PLATFORM, &[])
        .into_iter()
        .map(|trace| LedgerEntry {
            kind: trace.kind,
            target: "fremder-eintrag@example.org".into(),
            applied_at: 1,
        })
        .collect();
    fixture.write_state(None, ledger);
    let outcome = fixture.control(&mut integration, &shell).remove().unwrap();
    assert!(!outcome.errors.is_empty());
    assert!(
        fixture.dir().root().exists(),
        "the ledger stays for a retry"
    );
    assert!(task_fixtures::foreign_present(fixture.root()));
    task_fixtures::remove_foreign(fixture.root());
}

#[test]
fn desktop_task_user_disabled_autostart_respected() {
    let fixture = Fixture::new();
    let mut integration = fixture.integration();
    task_fixtures::switch_off_by_user(fixture.root(), &fixture.gui);
    let switched_off = task_fixtures::user_switch_off(fixture.root()).expect("switched off");
    let shell = FakeShell::default();
    for _ in 0..2 {
        let outcome = fixture
            .control(&mut integration, &shell)
            .enable(None)
            .unwrap();
        assert!(outcome.complete(), "{outcome:?}");
        assert_eq!(
            task_fixtures::user_switch_off(fixture.root()).as_deref(),
            Some(switched_off.as_str())
        );
        assert_eq!(integration.autostart(), AutostartState::DisabledBySystem);
        let report = status::gather(&fixture.dir(), &integration);
        assert_eq!(report.json()["autostart"], json!("disabled_by_system"));
    }
    let outcome = fixture.control(&mut integration, &shell).remove().unwrap();
    assert!(outcome.complete(), "{outcome:?}");
    assert_eq!(integration.autostart(), AutostartState::Off);
    task_fixtures::undo_switch_off(fixture.root());
}

static STARTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn record_start(gui: &Path) -> Result<(), String> {
    STARTED.lock().unwrap().push(gui.to_path_buf());
    Ok(())
}

fn slow_start(gui: &Path) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(300));
    record_start(gui)
}

fn started(gui: &Path) -> usize {
    STARTED
        .lock()
        .unwrap()
        .iter()
        .filter(|path| path.as_path() == gui)
        .count()
}

#[test]
fn desktop_task_halt_survives_gui_start_until_version_change() {
    let fixture = Fixture::new();
    let version = env!("CARGO_PKG_VERSION");
    fixture.write_state(Some(version), Vec::new());
    assert!(!launch::resume_wanted(&fixture.dir()));
    let resumed = launch::resume_with(fixture.gui.clone(), Some(fixture.dir()), record_start)
        .unwrap()
        .join()
        .unwrap();
    assert!(!resumed);
    assert_eq!(started(&fixture.gui), 0);
    let module = fixture.dir().read_state().unwrap().unwrap();
    assert_eq!(module.halted.unwrap().version, version);

    // Another program version resumes (Ä30).
    fixture.write_state(Some("0.0.0-andere-version"), Vec::new());
    assert!(launch::resume_wanted(&fixture.dir()));
    let resumed = launch::resume_with(fixture.gui.clone(), Some(fixture.dir()), record_start)
        .unwrap()
        .join()
        .unwrap();
    assert!(resumed);
    assert_eq!(started(&fixture.gui), 1);

    // "Neu starten" clears the halt of this version and starts the shell.
    fixture.write_state(Some(version), Vec::new());
    let mut integration = PlatformIntegration::for_task(fixture.root());
    let shell = FakeShell::default();
    let outcome = fixture.control(&mut integration, &shell).restart().unwrap();
    assert!(outcome.restarted, "{outcome:?}");
    assert!(fixture
        .dir()
        .read_state()
        .unwrap()
        .unwrap()
        .halted
        .is_none());
    assert_eq!(shell.started.borrow().len(), 1);
}

#[test]
fn desktop_task_resume_on_gui_start() {
    let fixture = Fixture::new();
    fixture.write_state(None, Vec::new());
    let begun = Instant::now();
    let handle = launch::resume_with(fixture.gui.clone(), Some(fixture.dir()), slow_start).unwrap();
    assert!(
        begun.elapsed() < Duration::from_millis(100),
        "the GUI start waited"
    );
    assert!(handle.join().unwrap());
    assert_eq!(started(&fixture.gui), 1);

    // Absent: nothing starts and nothing is created.
    let absent = Fixture::new();
    let begun = Instant::now();
    let handle = launch::resume_with(absent.gui.clone(), Some(absent.dir()), slow_start).unwrap();
    assert!(begun.elapsed() < Duration::from_millis(100));
    assert!(!handle.join().unwrap());
    assert_eq!(started(&absent.gui), 0);
    assert!(!absent.dir().root().exists());
}
