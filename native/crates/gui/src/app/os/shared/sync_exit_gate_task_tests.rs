//! RV1 fixtures: result delivery is distinct from actual worker completion.
use super::{App, ExitIntent, ReadyUpdate, SYNC_EXIT_WAIT};
use crate::app::sync_preview_types::PreviewApplyResult;
use crate::bisync::{Action, Preview};
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WORKER_TIMEOUT: Duration = Duration::from_secs(30);
const RESULT_ERROR: &str = "Y156 fixture: cancelled action remains recorded";

struct HeldWorker {
    app: App,
    cancel: Arc<AtomicBool>,
    rescued: Arc<AtomicBool>,
    publish: Sender<()>,
    release: Sender<()>,
    progress: Receiver<&'static str>,
    published: bool,
}

impl HeldWorker {
    fn new() -> Self {
        let mut app = App::new_for_copy_task();
        // The isolated constructor disables daemon/update startup. This
        // fixture exercises only the sync drain, gate and exit preflight.
        assert!(app.root_path.is_empty());
        app.error_msg = None;
        app.update_ready = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let rescued = Arc::new(AtomicBool::new(false));
        let worker_rescued = rescued.clone();
        let (publish, publish_rx) = bounded(1);
        let (release, release_rx) = bounded(1);
        let (progress_tx, progress) = bounded(2);
        let (result_tx, result_rx) = bounded(1);
        let worker = std::thread::Builder::new()
            .name("review-task-y156-held".into())
            .spawn(move || {
                let _ = progress_tx.send("started");
                if publish_rx.recv_timeout(WORKER_TIMEOUT).is_err() {
                    worker_rescued.store(true, Ordering::Release);
                    return;
                }
                let action = Action::CopyAtoB("held.txt".into());
                let _ = result_tx.send(PreviewApplyResult {
                    preview: Preview {
                        actions: vec![action.clone()],
                        ..Default::default()
                    },
                    action,
                    result: Err(RESULT_ERROR.into()),
                });
                let _ = progress_tx.send("published");
                // Cancel and result delivery do not finish a held final step.
                if release_rx.recv_timeout(WORKER_TIMEOUT).is_err() {
                    worker_rescued.store(true, Ordering::Release);
                }
            })
            .expect("spawn isolated Y156 worker");
        app.track_desktop_sync_worker(worker, cancel.clone());
        app.apply_one_rx = Some(result_rx);
        app.bisync_running = true;
        app.bisync_cancel = Some(cancel.clone());
        let fixture = Self {
            app,
            cancel,
            rescued,
            publish,
            release,
            progress,
            published: false,
        };
        assert_eq!(fixture.progress.recv_timeout(WORKER_TIMEOUT), Ok("started"));
        fixture
    }

    fn publish_result(&mut self) {
        if self.published {
            return;
        }
        self.publish.send(()).expect("publish Y156 result");
        assert_eq!(self.progress.recv_timeout(WORKER_TIMEOUT), Ok("published"));
        self.published = true;
    }

    fn cancel_and_expire_ui_wait(&mut self) {
        self.app.cancel_desktop_sync();
        self.app.sync_exit_gate.started = Some(
            Instant::now()
                .checked_sub(SYNC_EXIT_WAIT + Duration::from_secs(1))
                .expect("test clock can represent elapsed UI wait"),
        );
        assert!(self.cancel.load(Ordering::Acquire));
        assert!(self.app.desktop_sync_active());
    }

    fn frame(&mut self, ctx: &egui::Context, close_requested: bool) -> GateFrame {
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
        let mut intercepted = false;
        let output = ctx.run(input, |ctx| {
            // Same ordering as frame_update, without unrelated network work.
            self.app.drain_apply_one();
            intercepted |= self.app.sync_exit_gate_frame(ctx);
        });
        let commands = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("root viewport output")
            .commands
            .clone();
        GateFrame {
            intercepted,
            commands,
        }
    }

    fn complete(&mut self) {
        self.publish_result();
        self.release.send(()).expect("release held Y156 worker");
        let deadline = Instant::now() + WORKER_TIMEOUT;
        while self.app.desktop_sync_active() {
            assert!(Instant::now() < deadline, "Y156 worker completion deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(self.app.drain_desktop_sync_workers(), 0);
        assert!(
            !self.rescued.load(Ordering::Acquire),
            "worker needed rescue"
        );
    }
}

impl Drop for HeldWorker {
    fn drop(&mut self) {
        // Release both waits even when an assertion failed. Join only after
        // the actual worker has ended; never leave the fixture thread behind.
        let _ = self.publish.try_send(());
        let _ = self.release.try_send(());
        let deadline = Instant::now() + WORKER_TIMEOUT;
        while self.app.desktop_sync_active() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.app.drain_desktop_sync_workers();
    }
}

struct GateFrame {
    intercepted: bool,
    commands: Vec<egui::ViewportCommand>,
}

impl GateFrame {
    fn assert_held(&self) {
        assert!(self.intercepted);
        assert!(self.commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!self.commands.contains(&egui::ViewportCommand::Close));
    }
}

#[test]
#[ignore = "requires isolated RV1 app data and SMART_EXPLORER_COPY_PASTE_TASK=1"]
fn review_task_y156_close_waits_for_completion_after_cancel_and_deadline() {
    let mut fixture = HeldWorker::new();
    let ctx = egui::Context::default();
    fixture.frame(&ctx, true).assert_held();
    assert!(!fixture.cancel.load(Ordering::Acquire));
    fixture.cancel_and_expire_ui_wait();
    fixture.frame(&ctx, false).assert_held();
    assert!(fixture.app.desktop_sync_active());
    assert_eq!(fixture.app.drain_desktop_sync_workers(), 1);
    assert!(fixture.app.prepare_for_exit(false).is_err());
    assert!(!fixture.app.shutdown_prepared);
    assert!(fixture.app.apply_one_rx.is_some());
    fixture.complete();
    let finished = fixture.frame(&ctx, false);
    assert!(finished.intercepted);
    assert!(finished.commands.contains(&egui::ViewportCommand::Close));
    assert!(fixture.app.shutdown_prepared);
    assert!(!fixture.app.desktop_sync_active());
}

#[test]
#[ignore = "requires isolated RV1 app data and SMART_EXPLORER_COPY_PASTE_TASK=1"]
fn review_task_y156_results_drain_while_worker_is_still_alive() {
    let mut fixture = HeldWorker::new();
    let ctx = egui::Context::default();
    fixture.frame(&ctx, true).assert_held();
    fixture.cancel_and_expire_ui_wait();
    fixture.publish_result();
    assert!(fixture.app.apply_one_rx.is_some());
    fixture.frame(&ctx, false).assert_held();
    assert!(fixture.app.apply_one_rx.is_none());
    assert!(!fixture.app.bisync_running);
    assert!(fixture.app.bisync_cancel.is_none());
    assert!(fixture
        .app
        .error_msg
        .as_deref()
        .is_some_and(|e| e.contains(RESULT_ERROR)));
    assert_eq!(
        fixture
            .app
            .preview
            .as_ref()
            .expect("original plan retained")
            .actions,
        vec![Action::CopyAtoB("held.txt".into())]
    );
    assert!(fixture.app.desktop_sync_active());
    assert_eq!(fixture.app.drain_desktop_sync_workers(), 1);
    assert!(fixture.app.prepare_for_update_apply().is_err());
    assert!(!fixture.app.shutdown_prepared);
    fixture.complete();
    assert!(fixture
        .frame(&ctx, false)
        .commands
        .contains(&egui::ViewportCommand::Close));
}

#[test]
#[ignore = "requires isolated RV1 app data and SMART_EXPLORER_COPY_PASTE_TASK=1"]
fn review_task_y156_update_preflight_waits_for_actual_completion() {
    let directory = tempfile::tempdir().expect("isolated missing update payloads");
    let payload = |name: &str| {
        serde_json::json!({
            "path": directory.path().join(name),
            "sha256": "0".repeat(64)
        })
    };
    let bundle: crate::updater::StagedUpdate = serde_json::from_value(serde_json::json!({
        "schema": 0,
        "version": "review-y156",
        "app": payload("never-launch-app"),
        "helper": payload("never-launch-helper"),
        "cli": payload("never-launch-cli")
    }))
    .expect("actual StagedUpdate serde form");
    let mut fixture = HeldWorker::new();
    fixture.app.update_ready = Some(ReadyUpdate::Staged(bundle.clone()));
    fixture.app.request_sync_exit(ExitIntent::Update);
    let ctx = egui::Context::default();
    fixture.frame(&ctx, false).assert_held();
    assert!(
        fixture.app.error_msg.is_none(),
        "no active-worker preflight"
    );
    fixture.cancel_and_expire_ui_wait();
    fixture.frame(&ctx, false).assert_held();
    assert!(
        fixture.app.error_msg.is_none(),
        "deadline permits no preflight"
    );
    assert!(fixture.app.prepare_for_update_apply().is_err());
    assert!(!fixture.app.shutdown_prepared);
    fixture.complete();
    fixture.frame(&ctx, false).assert_held();
    assert!(!fixture.app.desktop_sync_active());
    assert!(
        !fixture.app.shutdown_prepared,
        "verification precedes shutdown"
    );
    assert_eq!(
        fixture.app.error_msg.as_deref(),
        Some(
            "Update-Staging ist nicht mehr gültig: Nicht unterstuetztes Staging-Manifest-Schema 0"
        )
    );
    assert!(matches!(
        fixture.app.update_ready.as_ref(),
        Some(ReadyUpdate::Staged(current)) if current == &bundle
    ));
    assert!(!directory.path().join("never-launch-helper").exists());
}
