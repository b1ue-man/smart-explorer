//! DSK-M3 process checks with the real GUI program (`cargo build --bin
//! smart_explorer`, or `SE_DESKTOP_TASK_GUI`): shell, surfaces, channel,
//! heartbeat, update stop and the read without service start. They run only
//! through the isolated remote task entrypoint (`SMART_EXPLORER_DESKTOP_TASK=1`,
//! one test thread, `--include-ignored`, R5 wiring of `run_if_requested`).
//! Each test points app data, configuration and runtime folders of this
//! process and of its children at a temporary root; the surfaces run with
//! the suite switch `SE_DESKTOP_TEST_HEADLESS=1`, so no display is needed.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::store::DesktopDir;
use super::supervisor::TEST_MONITORS;
use super::{launch, model_feed, platform};
use crate::daemon::EXIT_UPDATE_STOP;
use crate::desktop::core::protocol::{read_frame, SurfaceMessage, EXIT_CHANNEL_END};
use crate::desktop::core::state::{HaltReason, ModuleState, Part, PartSet, State, STATE_SCHEMA};

const HEADLESS: (&str, &str) = ("SE_DESKTOP_TEST_HEADLESS", "1");
const MONITOR: (&str, &str) = (TEST_MONITORS, "task-a");

fn gui_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("SE_DESKTOP_TASK_GUI") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap();
    let target = exe.parent().and_then(Path::parent).unwrap();
    let gui = target.join(format!("smart_explorer{}", std::env::consts::EXE_SUFFIX));
    assert!(
        gui.is_file(),
        "{} fehlt: zuerst `cargo build --bin smart_explorer`",
        gui.display()
    );
    gui
}

fn wait_for(limit: Duration, what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "{what}: nach {limit:?} nicht erreicht"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_exit(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Writes `daemon.stop` exactly like the updater helper.
fn write_marker(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "stop").unwrap();
}

struct TaskEnv {
    gui: PathBuf,
    saved: Vec<(String, Option<OsString>)>,
    children: Vec<Child>,
    _runtime: tempfile::TempDir,
    _root: tempfile::TempDir,
}

impl TaskEnv {
    fn new(switches: &[(&str, &str)]) -> TaskEnv {
        assert_eq!(
            std::env::var("SMART_EXPLORER_DESKTOP_TASK").as_deref(),
            Ok("1"),
            "isolated remote task entrypoint required"
        );
        let gui = gui_binary();
        let root = tempfile::tempdir().unwrap();
        // tempfile creates owner-only folders, as the daemon's lock needs.
        let runtime = tempfile::Builder::new()
            .prefix("run")
            .tempdir_in(root.path())
            .unwrap();
        let mut vars: Vec<(String, OsString)> = vec![
            ("XDG_DATA_HOME".into(), root.path().join("data").into()),
            ("XDG_CONFIG_HOME".into(), root.path().join("config").into()),
            ("XDG_RUNTIME_DIR".into(), runtime.path().into()),
            ("APPDATA".into(), root.path().join("appdata").into()),
            ("LOCALAPPDATA".into(), root.path().join("local").into()),
            (
                "SMART_EXPLORER_E2E_TEST_NAMESPACE".into(),
                format!("desktop_task_{}", std::process::id()).into(),
            ),
        ];
        for (name, value) in switches {
            vars.push(((*name).into(), (*value).into()));
        }
        let mut saved = Vec::new();
        for (name, value) in vars {
            saved.push((name.clone(), std::env::var_os(&name)));
            std::env::set_var(&name, value);
        }
        TaskEnv {
            gui,
            saved,
            children: Vec::new(),
            _runtime: runtime,
            _root: root,
        }
    }

    fn dir(&self) -> DesktopDir {
        DesktopDir::production()
    }

    fn marker(&self) -> PathBuf {
        crate::support_dirs::sync_data_dir().join("daemon.stop")
    }

    fn state(&self) -> Option<ModuleState> {
        self.dir().read_state().ok().flatten()
    }

    fn enable_surface(&self) {
        let dir = self.dir();
        dir.create().unwrap();
        dir.write_state(&ModuleState {
            schema: STATE_SCHEMA,
            state: State::Enabled,
            parts: PartSet::of(&[Part::Surface]),
            changed_at: 1,
            halted: None,
            ledger: Vec::new(),
        })
        .unwrap();
    }

    fn spawn(&mut self, args: &[&str]) -> usize {
        let child = Command::new(&self.gui)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.children.push(child);
        self.children.len() - 1
    }

    fn child(&mut self, index: usize) -> &mut Child {
        &mut self.children[index]
    }

    /// `shell.log` as `(unix seconds, event, detail)`.
    fn log(&self) -> Vec<(f64, String, String)> {
        std::fs::read_to_string(self.dir().log_path())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let at = parts.next()?.parse().ok()?;
                let event = parts.next()?.to_string();
                Some((at, event, parts.next().unwrap_or_default().to_string()))
            })
            .collect()
    }

    fn events(&self, name: &str) -> Vec<(f64, String)> {
        self.log()
            .into_iter()
            .filter(|(_, event, _)| event == name)
            .map(|(at, _, detail)| (at, detail))
            .collect()
    }

    /// PID of the first surface the shell started.
    fn surface_pid(&self) -> u32 {
        wait_for(Duration::from_secs(15), "Fläche gestartet", || {
            !self.events("surface_start").is_empty()
        });
        let (_, detail) = self.events("surface_start").remove(0);
        detail.split(' ').nth(1).unwrap().parse().unwrap()
    }

    fn alive(&self, pid: u32) -> bool {
        platform::process::alive_with_image(pid, &self.gui)
    }

    fn stop_shell(&mut self, index: usize) {
        self.dir().request_stop().unwrap();
        let status = wait_exit(self.child(index), Duration::from_secs(10)).expect("Wächter endet");
        assert_eq!(status.code(), Some(0));
    }

    fn surface(&self) -> (Child, std::process::ChildStdout) {
        let mut surface = Command::new(&self.gui)
            .args(["--desktop-surface", "--monitor", "task-a"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut output = surface.stdout.take().unwrap();
        let hello: Option<SurfaceMessage> = read_frame(&mut output).unwrap();
        assert!(
            matches!(&hello, Some(SurfaceMessage::Hello { monitor, .. }) if monitor == "task-a"),
            "{hello:?}"
        );
        (surface, output)
    }
}

impl Drop for TaskEnv {
    fn drop(&mut self) {
        // Ends a shell, its surfaces and a service of this root.
        let dir = self.dir();
        if dir.exists() {
            let _ = dir.request_stop();
        }
        let _ = crate::daemon::request_stop();
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        std::thread::sleep(Duration::from_millis(1500));
        for (name, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(&name, value),
                None => std::env::remove_var(&name),
            }
        }
    }
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_supervisor_restarts_after_crash() {
    let mut env = TaskEnv::new(&[MONITOR, HEADLESS, ("SE_DESKTOP_TEST_SURFACE_EXIT", "3")]);
    env.enable_surface();
    let shell = env.spawn(&["--desktop-shell"]);
    wait_for(Duration::from_secs(45), "angehalten", || {
        env.state().is_some_and(|state| state.halted.is_some())
    });
    let halt = env.state().unwrap().halted.unwrap();
    assert_eq!(halt.reason, HaltReason::CrashLoop);
    assert_eq!(halt.version, env!("CARGO_PKG_VERSION"));
    let starts = env.events("surface_start");
    let exits = env.events("surface_exit");
    assert_eq!(starts.len(), 3, "{:?}", env.log());
    assert_eq!(exits.len(), 3, "{:?}", env.log());
    assert!(exits.iter().all(|(_, detail)| detail.ends_with(" 3")));
    // Waits of 1 s and 5 s before the second and the third start (C5.5).
    assert!(starts[1].0 - exits[0].0 >= 0.95, "{:?}", env.log());
    assert!(starts[2].0 - exits[1].0 >= 4.95, "{:?}", env.log());
    assert_eq!(env.events("halt").len(), 1);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(env.events("surface_start").len(), 3, "start after the halt");
    // A GUI start does not revive a halt of this version (Ä30).
    assert!(!launch::resume_wanted(&env.dir()));
    // The halted shell waits for its notification, a restart request ends it.
    env.stop_shell(shell);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_supervisor_kills_hung_surface() {
    let mut env = TaskEnv::new(&[MONITOR, HEADLESS, ("SE_DESKTOP_TEST_NO_ALIVE", "1")]);
    env.enable_surface();
    let shell = env.spawn(&["--desktop-shell"]);
    let pid = env.surface_pid();
    let started_at = env.events("surface_start")[0].0;
    wait_for(Duration::from_secs(20), "Hänger erkannt", || {
        !env.events("surface_hung").is_empty()
    });
    let (hung_at, detail) = env.events("surface_hung").remove(0);
    assert!(detail.ends_with(&format!(" {pid}")), "{detail}");
    let silent = hung_at - started_at;
    assert!(
        (9.9..=13.0).contains(&silent),
        "{silent} s ohne Lebenszeichen"
    );
    wait_for(Duration::from_secs(3), "Fläche beendet", || {
        !env.alive(pid)
    });
    env.stop_shell(shell);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_surface_exits_on_channel_end() {
    let mut env = TaskEnv::new(&[MONITOR, HEADLESS]);
    // The channel ends: the surface ends with 76 within two seconds.
    let (mut surface, output) = env.surface();
    drop(surface.stdin.take());
    let closed = Instant::now();
    let status = wait_exit(&mut surface, Duration::from_secs(2)).expect("Kanalende");
    assert_eq!(status.code(), Some(EXIT_CHANNEL_END));
    assert!(closed.elapsed() <= Duration::from_secs(2));
    drop(output);

    // The shell is killed: its surface ends within two seconds.
    env.enable_surface();
    let shell = env.spawn(&["--desktop-shell"]);
    let pid = env.surface_pid();
    let child = env.child(shell);
    child.kill().unwrap();
    let _ = child.wait();
    let killed = Instant::now();
    wait_for(
        Duration::from_secs(3),
        "Fläche endet mit dem Wächter",
        || !env.alive(pid),
    );
    assert!(killed.elapsed() <= Duration::from_millis(2500));
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_update_stop_ends_shell_and_surfaces() {
    let mut env = TaskEnv::new(&[MONITOR, HEADLESS]);
    env.enable_surface();
    let marker = env.marker();
    let _ = std::fs::remove_file(&marker);
    let shell = env.spawn(&["--desktop-shell"]);
    let pid = env.surface_pid();
    // Both took their baseline; the request comes later (V8).
    std::thread::sleep(Duration::from_millis(1200));
    write_marker(&marker);
    let requested = Instant::now();
    let status = wait_exit(env.child(shell), Duration::from_secs(6)).expect("Stoppmerker");
    assert_eq!(status.code(), Some(EXIT_UPDATE_STOP));
    wait_for(Duration::from_secs(6), "Fläche endet", || !env.alive(pid));
    assert!(requested.elapsed() <= Duration::from_secs(6));
    assert_eq!(env.events("surface_start").len(), 1, "no restart");
    assert!(env.dir().read_shell().is_none());

    // A surface sees a new request itself and ends with 75.
    let (mut surface, _output) = env.surface();
    let before = std::fs::metadata(&marker).unwrap().modified().unwrap();
    write_marker(&marker);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&marker)
        .unwrap()
        .set_modified(before + Duration::from_secs(2))
        .unwrap();
    let status = wait_exit(&mut surface, Duration::from_secs(3)).expect("Fläche endet");
    assert_eq!(status.code(), Some(EXIT_UPDATE_STOP));

    // The next GUI start after the update starts the shell again (C5.6).
    let started = launch::resume_with(env.gui.clone(), Some(env.dir()), launch::start_shell)
        .unwrap()
        .join()
        .unwrap();
    assert!(started);
    wait_for(Duration::from_secs(5), "Wächter läuft wieder", || {
        platform::instance::is_running(&env.dir())
    });
    env.dir().request_stop().unwrap();
    wait_for(Duration::from_secs(10), "Wächter endet", || {
        !platform::instance::is_running(&env.dir())
    });
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_feed_never_starts_worker() {
    let mut env = TaskEnv::new(&[MONITOR, HEADLESS]);
    env.enable_surface();
    let shell = env.spawn(&["--desktop-shell"]);
    wait_for(Duration::from_secs(10), "Wächter läuft", || {
        env.dir().read_shell().is_some()
    });
    std::thread::sleep(Duration::from_secs(10));
    let sync = crate::support_dirs::sync_data_dir();
    assert!(
        !sync.join("daemon.ipc").exists(),
        "Hintergrunddienst gestartet"
    );
    assert!(!crate::daemon::is_running());
    assert!(matches!(crate::daemon::share_worker_peek(), Ok(None)));
    let runtime = env.dir().read_shell().expect("Wächter läuft");
    assert_eq!(runtime.share_state, "profiles");
    assert_eq!(
        runtime.share_state_reason.as_deref(),
        Some(model_feed::NO_SERVICE)
    );
    env.stop_shell(shell);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_peek_does_not_start_share_host() {
    let mut env = TaskEnv::new(&[]);
    let daemon = env.spawn(&["--sync-daemon", "--drive-restore"]);
    wait_for(Duration::from_secs(40), "Hintergrunddienst bereit", || {
        matches!(crate::daemon::share_worker_peek(), Ok(Some(_)))
    });
    // Longer than the snapshot's five-second reload (Annahme A41).
    let until = Instant::now() + Duration::from_secs(7);
    while Instant::now() < until {
        let snapshot = crate::daemon::share_worker_peek()
            .unwrap()
            .expect("Dienst läuft");
        assert!(!snapshot.running, "die Abfrage startete den Share-Host");
        std::thread::sleep(Duration::from_secs(1));
    }
    crate::daemon::request_stop().unwrap();
    let _ = wait_exit(env.child(daemon), Duration::from_secs(20));
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_connect_starts_worker() {
    let mut env = TaskEnv::new(&[MONITOR, HEADLESS, ("SE_DESKTOP_TEST_CONNECT", "1")]);
    env.enable_surface();
    assert!(matches!(crate::daemon::share_worker_peek(), Ok(None)));
    let shell = env.spawn(&["--desktop-shell"]);
    wait_for(
        Duration::from_secs(60),
        "Verbinden startet den Dienst",
        || matches!(crate::daemon::share_worker_peek(), Ok(Some(_))),
    );
    wait_for(Duration::from_secs(10), "Protokoll", || {
        !env.events("connect").is_empty()
    });
    env.stop_shell(shell);
    crate::daemon::request_stop().unwrap();
    wait_for(Duration::from_secs(20), "Dienst endet", || {
        matches!(crate::daemon::share_worker_peek(), Ok(None))
    });
}
