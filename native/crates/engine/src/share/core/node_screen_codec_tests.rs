//! H.264 and the host process between two real Iroh nodes on loopback
//! (SCR-M3). `screen_task_pair_bench_h264` needs Cisco's library
//! (`SE_SCREEN_TASK_OPENH264`, see `openh264_codec_tests.rs`) and encodes
//! in this process; `screen_task_worker_crash_keeps_daemon` runs the host
//! process of the development `se` (`SE_SCREEN_TASK_WORKER`).
use super::*;
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::os::host_worker_client::{WorkerBackend, WorkerProgram};
use crate::screen::os::openh264_codec::tests::{openh264_registry, store_with_openh264};

#[test]
#[ignore = "needs SE_SCREEN_TASK_OPENH264 (the suite downloads the pinned file)"]
fn screen_task_pair_bench_h264() {
    let _lock = crate::screen::os::openh264_codec::TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (dir, store) = store_with_openh264();
    let codecs = openh264_registry(&store);
    let pair = ScreenPair::new(Allow, Off);
    // The host offers the H.264 encoders of the same store (and Media
    // Foundation where present), encoding in this process.
    let backend = WorkerBackend::with(
        Arc::new(SyntheticBackend),
        WorkerProgram::InProcess,
        Some(dir.path().join("openh264")),
    );
    *pair.host.0.screen.backend.write().unwrap() = Arc::new(backend);
    let size = FrameSize::new(640, 360);
    let mut viewer = request(SessionMode::View, SourceId::Synthetic, size, 30);
    viewer.decoders = codecs.decoder_offers();
    let session = pair.open(&viewer, |_| {}).unwrap();
    let options = BenchOptions {
        codec: CodecPreference::H264,
        size,
        fps: 30,
        seconds: 3,
    };
    let report = pair
        .viewer
        .0
        .block_on(bench::run(session, options, &codecs))
        .unwrap();
    assert_eq!(report.codec, "h264", "{report:?}");
    assert_eq!(report.path, "direct", "{report:?}");
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(report.frames_shown >= 60, "{report:?}");
    assert!(report.keyframes >= 1, "{report:?}");
    assert!(
        report.static_psnr_db.is_none_or(|db| db >= 38.0),
        "{report:?}"
    );
}

// The crash hook exists in development builds only (`debug_assertions`).
#[cfg(debug_assertions)]
#[test]
#[ignore = "needs the development se (SE_SCREEN_TASK_WORKER)"]
fn screen_task_worker_crash_keeps_daemon() {
    let program = crate::screen::os::host_worker_client::tests::worker_program();
    let pair = ScreenPair::new(Allow, Off);
    // The host process of this session aborts after three pictures, like a
    // crashing codec (test hook of development builds).
    let crash = vec![(
        crate::screen::os::host_worker_main::CRASH_AFTER_ENV.to_string(),
        "3".to_string(),
    )];
    let crashing = WorkerBackend::with(
        Arc::new(SyntheticBackend),
        WorkerProgram::Fixed {
            path: program.clone(),
            env: crash,
        },
        None,
    );
    *pair.host.0.screen.backend.write().unwrap() = Arc::new(crashing);
    let session = pair.open(&test_picture(SessionMode::View), |_| {}).unwrap();
    let ended = pair
        .viewer
        .0
        .block_on(bench::run(session, options(5), &CodecRegistry::builtin()));
    match ended {
        Err(ViewerError::Refused(code)) => assert_eq!(code, "capture_failed"),
        Err(other) => panic!("expected capture_failed, got {other:?}"),
        Ok(report) => panic!("the crash went unnoticed: {report:?}"),
    }
    // The share host goes on: the next session with a sound host process
    // measures as before, and the registry is empty in between.
    let registry = pair.host.0.screen_registry().clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !registry.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(registry.is_empty());
    let sound = WorkerBackend::with(
        Arc::new(SyntheticBackend),
        WorkerProgram::Fixed {
            path: program,
            env: Vec::new(),
        },
        None,
    );
    *pair.host.0.screen.backend.write().unwrap() = Arc::new(sound);
    let report = bench_on(&pair, 2);
    assert_eq!(report.codec, "tiles");
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(report.frames_shown >= 20, "{report:?}");
}
