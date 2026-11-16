//! Task suite „Bildkette“ BK3 (plan 2026-11-14 B-27): the picture stream
//! with a built host process (`SE_SCREEN_TASK_WORKER`, as
//! `host_worker_tests`). Not registered yet (`docs/tests/register.tsv`).
use super::*;
use crate::screen::core::host_backend::PreparedSource;
use crate::screen::core::host_codec_adapters::{HostCodecPlan, HostContract, HostOutputPlan};
use crate::screen::core::negotiate::{CodecChoice, CodecKind};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::SourceId;

const SIZE: FrameSize = FrameSize::new(320, 180);

/// A host process of the development `se` with the test picture, opened
/// and activated as a session does it; `env` beside the daemon's.
fn open(env: &[(String, String)]) -> Box<dyn ScreenSource> {
    let admission = crate::screen::core::capture_test_support::session(None);
    let prepared = super::super::prepared::PreparedWorker::new(
        &super::super::tests::worker_program(),
        env,
        "fixture",
        admission.session(),
        &SourceId::Synthetic,
        Some(SIZE),
        false,
        admission.clone(),
        None,
    )
    .unwrap();
    let output = HostOutputPlan::new(HostContract::Legacy, SIZE, HostCodecPlan::Tiles).unwrap();
    let plan = prepared.capture_plan(&output).unwrap().unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    Box::new(prepared).activate(grant).unwrap()
}

fn request(frame_id: u64) -> StageRequest {
    StageRequest {
        frame_contract: None,
        visible_rects: None,
        frame_id,
        keyframe: frame_id == 1,
        target_bps: 1_000_000,
        size: SIZE,
        choice: CodecChoice {
            max_macroblocks_per_second: None,
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: FrameSize::MAX,
            max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
        },
        encoder_epoch: 1,
        fps: 30,
    }
}

/// A crash of the host process during the stream ends only its session:
/// the daemon gets `capture_failed` for the open credit, and a new session
/// starts a new host process that streams again.
#[test]
#[ignore = "needs the development se (SE_SCREEN_TASK_WORKER); Suite crates-task"]
fn bildkette_task_chain_host_process_crash_ends_only_its_session() {
    let crash = vec![(
        crate::screen::os::host_worker_main::CRASH_AFTER_ENV.to_owned(),
        "2".to_owned(),
    )];
    let mut source = open(&crash);
    for frame_id in 1..=2 {
        let stage = source.host_stage().unwrap();
        stage.set_stream_hint(crate::screen::core::host_stage::StreamHint {
            cadence_fps: 30,
            ahead: false,
        });
        let frame = stage.produce(&request(frame_id)).unwrap();
        assert!(
            matches!(frame, StageFrame::Encoded { .. }),
            "picture {frame_id} of the stream"
        );
        let report = stage.take_stream_report().unwrap();
        assert_eq!(report.frame_id, Some(frame_id));
    }
    let failure = source
        .host_stage()
        .unwrap()
        .produce(&request(3))
        .unwrap_err();
    assert_eq!(
        failure.code,
        ScreenErrorCode::CaptureFailed,
        "{}",
        failure.error
    );
    assert!(
        failure.error.to_string().contains("ended unexpectedly"),
        "{}",
        failure.error
    );
    drop(source);

    let mut again = open(&[]);
    let frame = again.host_stage().unwrap().produce(&request(1)).unwrap();
    assert!(matches!(frame, StageFrame::Encoded { keyframe: true, .. }));
}
