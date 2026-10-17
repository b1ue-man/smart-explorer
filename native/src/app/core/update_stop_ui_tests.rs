//! DSK-M2 GUI harness (`SMART_EXPLORER_GUI_TASK=1`, isolated app data): a
//! second window ends on the marker the updater helper writes, through the
//! normal close path, and ignores a marker that existed before it started.
use super::WindowUpdateStop;
use crate::app::App;
use crate::daemon::UpdateStopBaseline;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Faster than the production second so the harness stays short; the rule
/// and the close path are the production ones.
const POLL: Duration = Duration::from_millis(25);
/// Budget of DSK-M2: marker written → window closed and exit prepared.
const EXIT_BUDGET: Duration = Duration::from_secs(5);

/// Both tests write the real marker; never let them overlap.
static MARKER: Mutex<()> = Mutex::new(());

/// `<app data>/sync/daemon.stop`, the file the updater helper writes.
fn marker_path() -> PathBuf {
    crate::support_dirs::sync_data_dir().join("daemon.stop")
}

fn remove_marker() {
    match std::fs::remove_file(marker_path()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove stop marker: {error}"),
    }
}

/// Writes the marker exactly like `request_daemon_stop_marker` of the helper.
fn write_marker_like_helper() {
    std::fs::write(marker_path(), "stop").unwrap();
}

struct Frame {
    commands: Vec<egui::ViewportCommand>,
}

impl Frame {
    fn closes(&self) -> bool {
        self.commands.contains(&egui::ViewportCommand::Close)
    }
}

/// One frame with the head of `frame_update::update`: the update-stop check,
/// then the sync exit gate that sees the close request of the next frame.
fn frame(
    app: &mut App,
    stop: &WindowUpdateStop,
    ctx: &egui::Context,
    close_requested: bool,
) -> Frame {
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 900.0),
        )),
        ..Default::default()
    };
    if close_requested {
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
    }
    let output = ctx.run(input, |ctx| {
        if stop.take_close_request(ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        app.sync_exit_gate_frame(ctx);
    });
    let commands = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| viewport.commands.clone())
        .unwrap_or_default();
    Frame { commands }
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint"]
fn desktop_task_gui_window_exits_on_update_stop() {
    let _serial = MARKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    remove_marker();
    // The window that started the helper has ended; this is a second window.
    let mut app = App::new_for_gui_task();
    let stop = WindowUpdateStop::arm_with(UpdateStopBaseline::arm(), POLL).unwrap();
    let ctx = egui::Context::default();
    assert!(!frame(&mut app, &stop, &ctx, false).closes());

    let written = Instant::now();
    write_marker_like_helper();
    while !frame(&mut app, &stop, &ctx, false).closes() {
        assert!(
            written.elapsed() < EXIT_BUDGET,
            "window did not close on the update stop"
        );
        std::thread::sleep(POLL / 2);
    }
    // eframe reports the close in the next frame; nothing may cancel it.
    let accepted = frame(&mut app, &stop, &ctx, true);
    assert!(!accepted
        .commands
        .contains(&egui::ViewportCommand::CancelClose));
    // eframe then calls `on_exit`, which prepares the exit like a user close.
    eframe::App::on_exit(&mut app, None);
    assert!(app.shutdown_prepared);
    assert!(written.elapsed() <= EXIT_BUDGET);
    // A request is taken once: later frames do not repeat the close command.
    assert!(!stop.take_close_request(&ctx));
    remove_marker();
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint"]
fn desktop_task_stale_stop_marker_ignored_at_start() {
    let _serial = MARKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    remove_marker();
    // Left behind by an earlier update before this window started.
    write_marker_like_helper();
    let stale = std::fs::metadata(marker_path())
        .unwrap()
        .modified()
        .unwrap();
    let mut app = App::new_for_gui_task();
    let stop = WindowUpdateStop::arm_with(UpdateStopBaseline::arm(), POLL).unwrap();
    let ctx = egui::Context::default();
    let until = Instant::now() + POLL * 20;
    while Instant::now() < until {
        assert!(!frame(&mut app, &stop, &ctx, false).closes());
        std::thread::sleep(POLL / 2);
    }
    assert!(!app.shutdown_prepared);

    // A later request still closes the window. The explicit time keeps the
    // rewrite distinct on file systems with coarse modification times.
    write_marker_like_helper();
    std::fs::OpenOptions::new()
        .write(true)
        .open(marker_path())
        .unwrap()
        .set_modified(stale + Duration::from_secs(2))
        .unwrap();
    let rewritten = Instant::now();
    while !frame(&mut app, &stop, &ctx, false).closes() {
        assert!(
            rewritten.elapsed() < EXIT_BUDGET,
            "window ignored a new request"
        );
        std::thread::sleep(POLL / 2);
    }
    remove_marker();
}
