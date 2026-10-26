//! The Android host's Rust side without a device (SCR-M8): a session waits
//! for the app's system dialog and ends on its refusal, H.264 pictures are
//! taken per encoder epoch and key frame, stills become tiles, and input
//! reaches the accessibility service only while it is on and a full stream
//! runs. The app's side runs in `ScreenTaskTest.kt`.
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use super::bridge_backend::{BridgeBackend, BridgeIndicator, HOST_APP_NOT_RUNNING};
use super::bridge_channels::{self as channels, Phase, FLAG_KEYFRAME, FLAG_STILL};
use super::bridge_source::{decode_still, epoch_flags};
use crate::screen::core::host_backend::{ScreenHostBackend, ScreenSource, SessionIndicator};
use crate::screen::core::host_stage::{StageFrame, StageRequest};
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::ScreenErrorCode;

fn fresh() -> MutexGuard<'static, ()> {
    let guard = channels::TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    channels::reset_for_tests();
    guard
}

fn register_app(accessibility: &str) {
    channels::set_prereqs(|prereqs| {
        prereqs.registered = true;
        prereqs.accessibility = accessibility.into();
        prereqs.display = Some((1080, 2400));
        prereqs.encoders = vec![crate::screen::core::negotiate::CodecOffer::h264(H264Profile::High, true), crate::screen::core::negotiate::CodecOffer::h264(H264Profile::ConstrainedBaseline, true)];
    });
}

fn request(codec: CodecKind, epoch: u64, keyframe: bool) -> StageRequest {
    StageRequest {
        frame_contract: None,
        visible_rects: None,
        frame_id: epoch * 10,
        keyframe,
        target_bps: 4_000_000,
        size: FrameSize::new(540, 1200),
        choice: CodecChoice {
            max_macroblocks_per_second: None,
            codec,
            profile: (codec == CodecKind::H264).then_some(H264Profile::High),
            encoder_hardware: true,
            decoder_hardware: true,
            max_size: FrameSize::MAX,
            max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
        },
        encoder_epoch: epoch,
        fps: 30,
    }
}

fn ready(id: u64, projection: bool) {
    let phase = Phase::Ready {
        width: 1080,
        height: 2400,
        encoder: "c2.qti.avc.encoder".into(),
        hardware: true,
        projection,
    };
    channels::ready(id, phase).unwrap();
}

fn drain(id: u64) -> Vec<String> {
    std::iter::from_fn(|| channels::control(id, Duration::from_millis(10)))
        .take(16)
        .collect()
}

#[test]
fn screen_task_android_stage_waits_for_consent_and_ends_on_refusal() {
    let _guard = fresh();
    let backend = BridgeBackend;
    let monitor = SourceId::Monitor("0".into());
    // Without the app nothing is shown and no source opens.
    assert!(!BridgeIndicator::new(&Arc::new(ScreenRegistry::default())).available());
    let refused = backend.open_source(&monitor, None).err().unwrap();
    assert_eq!(refused.reason, HOST_APP_NOT_RUNNING);
    register_app("off");
    let mut source = backend.open_source(&monitor, None).unwrap();
    let stage = source.host_stage().unwrap();
    let paused = stage.produce(&request(CodecKind::H264, 1, true)).unwrap();
    assert_eq!(paused, StageFrame::Paused(SessionState::WaitingForPortal));
    let id = channels::channels()[0].id;
    channels::ended(id, "consent_denied").unwrap();
    let error = stage
        .produce(&request(CodecKind::H264, 1, true))
        .err()
        .unwrap();
    assert_eq!(error.code, ScreenErrorCode::ApprovalDenied);
    drop(source);
    assert!(channels::channels().is_empty());
    assert!(drain(id).iter().any(|message| message.contains("\"stop\"")));
}

#[test]
fn screen_task_android_stage_configures_and_takes_key_frames() {
    let _guard = fresh();
    register_app("off");
    let mut source = BridgeBackend
        .open_source(&SourceId::Monitor("0".into()), None)
        .unwrap();
    let id = channels::channels()[0].id;
    ready(id, true);
    // The app's encoder of epoch 2 answers after `configure`; a stale key
    // frame of epoch 1 and a delta before the key frame are dropped.
    let key = epoch_flags(2) | FLAG_KEYFRAME;
    let app = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            channels::push(id, b"old", epoch_flags(1) | FLAG_KEYFRAME),
            0
        );
        assert_eq!(channels::push(id, b"delta", epoch_flags(2)), 0);
        assert_eq!(channels::push(id, b"\0\0\0\x01key", key), 0);
    });
    let stage = source.host_stage().unwrap();
    match stage.produce(&request(CodecKind::H264, 2, true)).unwrap() {
        StageFrame::Encoded {
            records, keyframe, ..
        } => {
            assert!(keyframe);
            assert_eq!(records, vec![b"\0\0\0\x01key".to_vec()]);
        }
        other => panic!("expected a key frame, got {other:?}"),
    }
    app.join().unwrap();
    let sent = drain(id);
    assert!(sent[0].contains("\"op\":\"configure\""), "{sent:?}");
    assert!(sent[0].contains("\"width\":540") && sent[0].contains("\"profile\":\"high\""));
    // Nothing new: unchanged, no request to the app.
    let idle = stage.produce(&request(CodecKind::H264, 2, false)).unwrap();
    assert!(matches!(idle, StageFrame::Unchanged { .. }));
    assert!(drain(id).is_empty());
    // An empty or oversized picture and an unknown session are refused.
    assert_eq!(channels::push(id, b"", key), channels::PUSH_INVALID);
    assert_eq!(channels::push(id + 99, b"x", key), channels::PUSH_UNKNOWN);
}

#[test]
fn screen_task_android_still_decodes_and_tiles() {
    let _guard = fresh();
    register_app("on");
    let mut still = Vec::new();
    still.extend_from_slice(&4u32.to_le_bytes());
    still.extend_from_slice(&2u32.to_le_bytes());
    still.extend(std::iter::repeat([10u8, 20, 30, 0]).take(8).flatten());
    let image = decode_still(&still).unwrap();
    assert_eq!(&image.data()[..4], &[30, 20, 10, 0xFF]);
    assert!(decode_still(&still[..12]).is_err());
    let mut source = BridgeBackend
        .open_source(&SourceId::Monitor("0".into()), None)
        .unwrap();
    let id = channels::channels()[0].id;
    let mut full = Vec::new();
    full.extend_from_slice(&1080u32.to_le_bytes());
    full.extend_from_slice(&2400u32.to_le_bytes());
    full.resize(8 + 1080 * 2400 * 4, 0x80);
    ready(id, false);
    assert_eq!(channels::push(id, &full, FLAG_STILL), 0);
    let stage = source.host_stage().unwrap();
    match stage.produce(&request(CodecKind::Tiles, 3, true)).unwrap() {
        StageFrame::Encoded {
            records, encoder, ..
        } => {
            assert_eq!(encoder.name, "tiles");
            assert!(!records.is_empty());
        }
        other => panic!("expected tiles, got {other:?}"),
    }
    assert!(drain(id)
        .iter()
        .any(|message| message.contains("\"op\":\"still\"")));
    // A still preview that turns into a full stream asks for the system dialog.
    let paused = stage.produce(&request(CodecKind::H264, 4, true)).unwrap();
    assert_eq!(paused, StageFrame::Paused(SessionState::WaitingForPortal));
    assert!(drain(id)
        .iter()
        .any(|message| message.contains("\"op\":\"consent\"")));
}

#[test]
fn screen_task_android_input_needs_accessibility_and_stream() {
    let _guard = fresh();
    let tap = [InputEvent::Text {
        text: "hallo".into(),
    }];
    register_app("off");
    assert_eq!(
        BridgeBackend.inject(&tap),
        Err(ControlBlock::ControlUnavailable)
    );
    assert!(!BridgeBackend.input().touch);
    register_app("on");
    // On, but no full stream runs: still refused.
    assert_eq!(
        BridgeBackend.inject(&tap),
        Err(ControlBlock::ControlUnavailable)
    );
    let _source = BridgeBackend
        .open_source(&SourceId::Monitor("0".into()), None)
        .unwrap();
    let id = channels::channels()[0].id;
    ready(id, true);
    assert!(BridgeBackend.input().touch);
    assert_eq!(BridgeBackend.inject(&tap), Ok(()));
    let input = drain(channels::INPUT_CHANNEL);
    assert_eq!(input.len(), 1);
    assert!(input[0].contains("\"op\":\"input\"") && input[0].contains("hallo"));
    assert_eq!(
        BridgeBackend.limits(),
        [
            "consent_each_session",
            "no_key_events",
            "still_max_3_per_second"
        ]
    );
}
