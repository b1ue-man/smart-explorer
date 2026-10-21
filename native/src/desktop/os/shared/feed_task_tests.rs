//! DSK-M4 process checks with the real background service of the GUI program
//! (`cargo build --bin smart_explorer`, or `SE_DESKTOP_TASK_GUI`): the shell's
//! reads never take the GUI's Share events (F8) and never lift the
//! restriction of a service started for the drive restore (V11). They run
//! only through the isolated remote task entrypoint
//! (`SMART_EXPLORER_DESKTOP_TASK=1`, one test thread, `--include-ignored`);
//! app data, configuration and runtime folders point at a temporary root.
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::daemon::ShareWorkerSnapshot;
use crate::share::{ShareCmd, ShareEvent};

/// The event the GUI must still get after the shell read it.
const STOPPED: &str = "Share-Worker getrennt";
const FEED_TICK: Duration = super::super::model_feed::FEED_TICK;

fn gui_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("SE_DESKTOP_TASK_GUI") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap();
    let target = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let gui = target.join(format!("smart_explorer{}", std::env::consts::EXE_SUFFIX));
    assert!(
        gui.is_file(),
        "{} fehlt: zuerst `cargo build --bin smart_explorer`",
        gui.display()
    );
    gui
}

/// An isolated user: own app data, configuration and runtime folders.
struct Isolated {
    saved: Vec<(String, Option<OsString>)>,
    service: Option<Child>,
    _runtime: tempfile::TempDir,
    _root: tempfile::TempDir,
}

impl Isolated {
    fn new() -> Isolated {
        assert_eq!(
            std::env::var("SMART_EXPLORER_DESKTOP_TASK").as_deref(),
            Ok("1"),
            "isolated remote task entrypoint required"
        );
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::Builder::new()
            .prefix("run")
            .tempdir_in(root.path())
            .unwrap();
        let vars: [(&str, OsString); 6] = [
            ("XDG_DATA_HOME", root.path().join("data").into()),
            ("XDG_CONFIG_HOME", root.path().join("config").into()),
            ("XDG_RUNTIME_DIR", runtime.path().into()),
            ("APPDATA", root.path().join("appdata").into()),
            ("LOCALAPPDATA", root.path().join("local").into()),
            (
                "SMART_EXPLORER_E2E_TEST_NAMESPACE",
                format!("desktop_task_feed_{}", std::process::id()).into(),
            ),
        ];
        let mut saved = Vec::new();
        for (name, value) in vars {
            saved.push((name.to_string(), std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        Isolated {
            saved,
            service: None,
            _runtime: runtime,
            _root: root,
        }
    }

    /// Starts the background service of the GUI program and waits until it
    /// answers the read without start.
    fn start_service(&mut self, args: &[&str]) {
        let child = Command::new(gui_binary())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.service = Some(child);
        let deadline = Instant::now() + Duration::from_secs(40);
        while !matches!(crate::daemon::share_worker_peek(), Ok(Some(_))) {
            assert!(
                Instant::now() < deadline,
                "Hintergrunddienst bereit: nach 40s nicht erreicht; {}",
                self.report()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// What a failed wait shows: whether the process still runs, what the
    /// worker answers and the tail of the worker log (the service has no
    /// output of its own).
    fn report(&mut self) -> String {
        let process = match self.service.as_mut().map(Child::try_wait) {
            Some(Ok(Some(status))) => format!("beendet mit {status}"),
            Some(Ok(None)) => "läuft".to_string(),
            Some(Err(error)) => format!("Zustand unbekannt: {error}"),
            None => "nicht gestartet".to_string(),
        };
        let answer = crate::daemon::share_worker_peek().map(|snapshot| snapshot.map(|_| "Antwort"));
        format!(
            "Dienstprozess {process}; Abfrage {answer:?}; Protokoll:\n{}",
            crate::daemon::read_log_tail(60)
        )
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        let _ = crate::daemon::request_stop();
        if let Some(mut service) = self.service.take() {
            let deadline = Instant::now() + Duration::from_secs(20);
            while matches!(service.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = service.kill();
            let _ = service.wait();
        }
        for (name, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(&name, value),
                None => std::env::remove_var(&name),
            }
        }
    }
}

fn has_stop_event(snapshot: &ShareWorkerSnapshot) -> bool {
    snapshot
        .events
        .iter()
        .any(|event| matches!(event, ShareEvent::Status(text) if text == STOPPED))
}

fn peek() -> ShareWorkerSnapshot {
    crate::daemon::share_worker_peek()
        .unwrap()
        .expect("Hintergrunddienst läuft")
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_share_events_not_drained() {
    let mut env = Isolated::new();
    env.start_service(&["--sync-daemon"]);
    // A UI event of the GUI: stopping the Share worker queues one.
    crate::daemon::share_command(ShareCmd::Stop).unwrap();
    // The shell reads it for several ticks and leaves it in place.
    for _ in 0..3 {
        let snapshot = peek();
        assert!(has_stop_event(&snapshot), "{:?}", snapshot.events);
        std::thread::sleep(FEED_TICK);
    }
    // The GUI drains while the shell keeps reading: the GUI gets the event.
    let reading = AtomicBool::new(true);
    let (drained, reads) = std::thread::scope(|scope| {
        let shell = scope.spawn(|| {
            let mut reads = 0u32;
            while reading.load(Ordering::Acquire) {
                if matches!(crate::daemon::share_worker_peek(), Ok(Some(_))) {
                    reads += 1;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            reads
        });
        std::thread::sleep(Duration::from_millis(300));
        let drained = crate::daemon::drain_share_worker_events().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        reading.store(false, Ordering::Release);
        (drained, shell.join().unwrap())
    });
    assert!(reads > 0, "the shell did not read while the GUI drained");
    assert!(has_stop_event(&drained), "{:?}", drained.events);
    // The GUI took it: the shell no longer sees it.
    let after = peek();
    assert!(!has_stop_event(&after), "{:?}", after.events);
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_feed_reads_keep_drive_restore_restricted() {
    let mut env = Isolated::new();
    env.start_service(&["--sync-daemon", "--drive-restore"]);
    // The reads of the shell's ticks, longer than the snapshot's five-second
    // reload: device tree and vault sites, then one long poll.
    let until = Instant::now() + Duration::from_secs(7);
    let mut revision = 0;
    while Instant::now() < until {
        if let Ok(Some(answer)) = crate::daemon::drive_devices_peek() {
            revision = super::parse_device_tree(&answer).map_or(0, |tree| tree.revision);
        }
        let _ = crate::daemon::vault_status_peek();
        let _ = peek();
        std::thread::sleep(FEED_TICK);
    }
    let _ = crate::daemon::drive_devices_watch_peek(revision);
    let _ = peek();
    let log = crate::daemon::read_log_tail(400);
    assert!(
        log.contains("daemon started for the drive restore"),
        "kein Anmeldestart: {log}"
    );
    assert!(
        !log.contains("drive-restore daemon promoted"),
        "eine Leseanfrage der Fläche hob die Einschränkung auf: {log}"
    );
}
