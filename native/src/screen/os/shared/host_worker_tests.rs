//! The host process with a built `se` (SCR-M3). `SE_SCREEN_TASK_WORKER`
//! names the development `se` the suite built (`cargo build --bin se`);
//! without it the test looks next to the test binary (`target/debug/se`).
use super::*;
use crate::screen::core::negotiate::{CodecChoice, CodecKind};

/// The `se` that runs host processes in the task tests.
pub(crate) fn worker_program() -> PathBuf {
    if let Ok(path) = std::env::var("SE_SCREEN_TASK_WORKER") {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) { "se.exe" } else { "se" };
    let current = std::env::current_exe().unwrap();
    let candidate = current
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join(name))
        .unwrap();
    assert!(
        candidate.is_file(),
        "SE_SCREEN_TASK_WORKER names no program and {candidate:?} is missing"
    );
    candidate
}

fn tiles() -> CodecChoice {
    CodecChoice {
            max_macroblocks_per_second: None,
        codec: CodecKind::Tiles,
        profile: None,
        encoder_hardware: false,
        decoder_hardware: false,
        max_size: FrameSize::MAX,
        max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
    }
}

#[test]
#[ignore = "needs the development se (SE_SCREEN_TASK_WORKER)"]
fn screen_task_host_worker_exits_on_update_stop() {
    // The host process reads its stop marker from its own app data.
    let data = tempfile::tempdir().unwrap();
    let size = FrameSize::new(320, 180);
    let admission = crate::screen::core::capture_test_support::session(None);
    let data_key = if cfg!(windows) {"APPDATA"} else {"XDG_DATA_HOME"};
    let env = vec![(data_key.to_owned(), data.path().to_string_lossy().into_owned())];
    let prepared = prepared::PreparedWorker::new(&worker_program(), &env, "fixture",
        admission.session(), &SourceId::Synthetic, Some(size), false, admission.clone(), None).unwrap();
    let output = crate::screen::core::host_codec_adapters::HostOutputPlan::new(
        crate::screen::core::host_codec_adapters::HostContract::Legacy, size,
        crate::screen::core::host_codec_adapters::HostCodecPlan::Tiles).unwrap();
    use crate::screen::core::host_backend::PreparedSource;
    let plan = prepared.capture_plan(&output).unwrap().unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    let link = prepared.shared_link();
    let mut source = Box::new(prepared).activate(grant).unwrap();
    assert_eq!(source.size(), size);
    let request = StageRequest {
        frame_contract: None,
        visible_rects: None,
        frame_id: 1,
        keyframe: true,
        target_bps: 1_000_000,
        size,
        choice: tiles(),
        encoder_epoch: 1,
        fps: 30,
    };
    let first = source.host_stage().unwrap().produce(&request).unwrap();
    assert!(matches!(first, StageFrame::Encoded { keyframe: true, .. }));

    // A new update-stop request, as the updater helper writes it (V8).
    let sync = data.path().join("smart_explorer").join("sync");
    std::fs::create_dir_all(&sync).unwrap();
    std::fs::write(sync.join("daemon.stop"), "stop").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = link.lock().unwrap().child.as_mut().unwrap().try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the host process did not end on the update stop"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(crate::daemon::EXIT_UPDATE_STOP));

    // The session learns why: `host_shutdown`, not a crash.
    let next = StageRequest {
        frame_id: 2,
        keyframe: false,
        ..request
    };
    let failure = source.host_stage().unwrap().produce(&next).unwrap_err();
    assert_eq!(
        failure.code,
        ScreenErrorCode::HostShutdown,
        "{}",
        failure.error
    );
}
