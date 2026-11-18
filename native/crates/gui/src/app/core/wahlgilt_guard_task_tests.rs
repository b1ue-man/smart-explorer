//! Finding 3 of the user rule of 2026-11-18: a named codec that brings no
//! picture is neither given up nor switched; the window shows the error
//! with the guard's reason. „Verlustfrei“ goes to the tiles it chose
//! without giving anything up. (Fix of 2026-11-18 „Kacheln immer“: a
//! give-up belongs to its session, `Shared::give_up_codec`; the check of
//! the process-wide mark became a check of the session's.)
use std::io;
use std::sync::{Arc, Mutex};

use super::DecodeState;
use crate::screen::core::decoder_health::FAILURE_LIMIT;
use crate::screen::core::host_backend::ScreenDecoder;
use crate::screen::core::negotiate::{CodecKind, CodecSetting};
use crate::screen::core::wire_binary::GopHeader;
use crate::screen::core::wire_control::{SetMode, StreamMode, ViewerFrame};

pub(super) type Sent = Arc<Mutex<Vec<ViewerFrame>>>;

/// A decode thread's state for `codec` without a host (also the kachelfix
/// tests of 2026-11-18).
pub(super) fn state(
    codec: CodecSetting,
) -> (
    DecodeState,
    Arc<super::super::super::screen_session::Shared>,
    Sent,
) {
    let shared = super::super::super::screen_session::Shared::new(false, 0);
    let mode = SetMode {
        mode: StreamMode::Full,
        source: None,
        size: None,
        fps: None,
        sharpness: false,
        lossless: false,
    };
    shared.set_opening(mode, codec);
    let sent: Sent = Arc::default();
    let log = sent.clone();
    let send = Arc::new(move |frame: &ViewerFrame| {
        log.lock().unwrap().push(frame.clone());
        Ok::<(), io::Error>(())
    });
    let decoder: super::super::super::screen_session::DecoderOpener =
        Arc::new(|_header: &GopHeader| {
            Err::<Box<dyn ScreenDecoder>, io::Error>(io::Error::other("no decoder"))
        });
    (
        DecodeState::new(shared.clone(), decoder, send, false),
        shared,
        sent,
    )
}

#[test]
fn wahlgilt_task_named_codec_without_picture_shows_the_error() {
    let (mut decode, shared, sent) = state(CodecSetting::H264);
    for _ in 0..FAILURE_LIMIT {
        decode.guard_failure(CodecKind::H264);
    }
    let notice = shared.info().notice.clone().expect("the error is shown");
    assert!(
        notice.starts_with("H.264 liefert auf diesem Gerät kein Bild ("),
        "{notice}"
    );
    assert!(notice.contains("Gewählt ist „H.264“"), "{notice}");
    assert!(
        sent.lock().unwrap().is_empty(),
        "no SET_DECODERS, no SET_MODE"
    );
    assert!(
        shared.give_up_codec(CodecKind::H264).is_some(),
        "not given up"
    );

    // „Verlustfrei“: the tiles it chose, nothing marked.
    let (mut decode, shared, sent) = state(CodecSetting::Lossless);
    for _ in 0..FAILURE_LIMIT {
        decode.guard_failure(CodecKind::Hevc);
    }
    let frames = sent.lock().unwrap().clone();
    assert!(frames
        .iter()
        .any(|frame| matches!(frame, ViewerFrame::SetMode(set) if set.lossless)));
    assert!(shared.give_up_codec(CodecKind::Hevc).is_some());
}
