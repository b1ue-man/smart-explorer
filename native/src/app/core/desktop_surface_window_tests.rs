//! DSK-M4 GUI harness, second half: the surface process in its window test
//! mode (`SE_DESKTOP_TEST_WINDOWED=1`) against a channel of the test acting
//! as shell. It needs a display (Xvfb under Linux) and the built
//! `smart_explorer` (or `SE_DESKTOP_TASK_GUI`); the test of
//! `desktop_surface_task_tests.rs` calls it.
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::task_tests::{config, model, MONITOR};
use crate::desktop::core::protocol::{
    read_frame, write_frame, ShellMessage, StopReason, SurfaceMessage, PROTOCOL_VERSION,
};
use crate::desktop::os::feed_sources::TEST_DESKTOP_DIR;

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

/// Starts the surface as an ordinary window, sends greeting, configuration
/// and model, waits for two heartbeats of drawn frames and ends it with
/// `stop`.
pub(super) fn windowed_process() {
    let desktop = tempfile::tempdir().unwrap();
    std::fs::write(desktop.path().join("notiz.txt"), b"x").unwrap();
    let mut surface = Command::new(gui_binary())
        .args(["--desktop-surface", "--monitor", MONITOR])
        .env("SE_DESKTOP_TEST_WINDOWED", "1")
        .env(TEST_DESKTOP_DIR, desktop.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut output = surface.stdout.take().unwrap();
    let mut input = surface.stdin.take().unwrap();
    let hello: Option<SurfaceMessage> = read_frame(&mut output).unwrap();
    assert!(
        matches!(&hello, Some(SurfaceMessage::Hello { monitor, .. }) if monitor == MONITOR),
        "{hello:?}"
    );
    let (sender, messages) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(Some(message)) = read_frame::<_, SurfaceMessage>(&mut output) {
            if sender.send(message).is_err() {
                return;
            }
        }
    });
    let greeting = ShellMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: MONITOR.into(),
    };
    write_frame(&mut input, &greeting).unwrap();
    write_frame(&mut input, &ShellMessage::Config { config: config() }).unwrap();
    let shell_model = ShellMessage::Model {
        rev: 1,
        model: Box::new(model()),
    };
    write_frame(&mut input, &shell_model).unwrap();
    input.flush().unwrap();
    // The window draws the view: two heartbeats come from drawn frames.
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut alive = 0;
    while alive < 2 {
        let left = deadline.saturating_duration_since(Instant::now());
        match messages.recv_timeout(left) {
            Ok(SurfaceMessage::Alive) => alive += 1,
            Ok(_) => {}
            Err(error) => panic!("kein Bild der Fläche im Fenstermodus: {error}"),
        }
    }
    let stop = ShellMessage::Stop {
        reason: StopReason::Disable,
    };
    write_frame(&mut input, &stop).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = surface.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Fläche endet nicht auf `stop`");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0));
}
