//! HEVC and AV1 of the Android host (plan 2026-11-14 „Bildkette“ BK7b, B-26): the core names the
//! negotiated codec to the app, offers HEVC and AV1 only while the app's encoder is not refused,
//! and the stream is checked before MediaCodec sees it.
use super::*;

/// The `configure` message the app receives for `codec` in epoch 2 (the app thread answers with the
/// key frame the stage waits for).
fn configure_for(codec: CodecKind) -> serde_json::Value {
    let _guard = fresh();
    register_app("off");
    let (prepared, admission) = prepare();
    let id = channels::channels()[0].id;
    ready(id, true);
    let mut source = activate(prepared, &admission, CodecKind::H264);
    let grant = source.capture_grant().unwrap();
    let app = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            push(id, &grant, b"\0\0\0\x01key", epoch_flags(2) | FLAG_KEYFRAME),
            0
        );
    });
    let stage = source.host_stage().unwrap();
    assert!(matches!(
        stage.produce(&request(codec, 2, true)).unwrap(),
        StageFrame::Encoded { keyframe: true, .. }
    ));
    app.join().unwrap();
    let sent = drain(id);
    serde_json::from_str(&sent[0]).unwrap()
}

#[test]
fn bildkette_task_android_core_passes_the_negotiated_codec_to_the_app() {
    // H.264 keeps its message: `h264` with the profile name the app already reads.
    let h264 = configure_for(CodecKind::H264);
    assert_eq!(h264["op"], "configure");
    assert_eq!(h264["codec"], "h264");
    assert_eq!(h264["profile"], "high");
    // HEVC and AV1 4:2:0: the codec key and profile `main`, never a hybrid pair.
    for (codec, key) in [(CodecKind::Hevc, "hevc"), (CodecKind::Av1, "av1")] {
        let message = configure_for(codec);
        assert_eq!(message["codec"], key);
        assert_eq!(message["profile"], "main");
        assert_eq!(message["hybrid"], false);
        assert_eq!(message["width"], 540);
        assert_eq!(message["epoch"], 2);
    }
}

#[test]
fn bildkette_task_android_offers_follow_the_apps_report_and_its_refusals() {
    use crate::screen::core::negotiate::{CodecOffer, MAX_CODEC_OFFERS};
    let _guard = fresh();
    register_app("off");
    let hevc = CodecOffer::video(CodecKind::Hevc, false, true);
    let av1 = CodecOffer::video(CodecKind::Av1, false, true);
    let h264 = CodecOffer::h264(H264Profile::High, true);
    channels::set_prereqs(|prereqs| prereqs.encoders = vec![h264, hevc, av1]);
    // H.264, the tiles, then HEVC and AV1: `wire_hello_codecs` splits the list at the tiles.
    assert_eq!(
        BridgeBackend.encoders(),
        vec![h264, CodecOffer::tiles(), hevc, av1]
    );
    // An HEVC encoder the app failed to open leaves the offers; the others stay.
    channels::refuse_codec(CodecKind::Hevc);
    channels::refuse_codec(CodecKind::H264);
    assert_eq!(
        BridgeBackend.encoders(),
        vec![h264, CodecOffer::tiles(), av1]
    );
    // Many H.264 offers never push HEVC and AV1 out of the list.
    let many: Vec<CodecOffer> = (0..MAX_CODEC_OFFERS + 4)
        .map(|index| {
            CodecOffer::h264(H264Profile::High, true)
                .with_max(FrameSize::new(100 + index as u32, 100))
        })
        .collect();
    channels::set_prereqs(|prereqs| {
        prereqs.refused.clear();
        prereqs.encoders = many.iter().copied().chain([hevc, av1]).collect();
    });
    let offers = BridgeBackend.encoders();
    assert_eq!(offers.len(), MAX_CODEC_OFFERS + 2);
    assert_eq!(offers[MAX_CODEC_OFFERS - 1], CodecOffer::tiles());
    assert_eq!(&offers[MAX_CODEC_OFFERS..], &[hevc, av1]);
}

#[test]
fn bildkette_task_android_a_refused_encoder_falls_back_without_ending_the_session() {
    let _guard = fresh();
    register_app("off");
    let (prepared, admission) = prepare();
    let id = channels::channels()[0].id;
    ready(id, true);
    let mut source = activate(prepared, &admission, CodecKind::H264);
    let grant = source.capture_grant().unwrap();
    let app = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        // The app's `encoderFailed`: flag 0x8, the reason as text, in the epoch of the configure.
        assert_eq!(push(id, &grant, b"HEVC refused", epoch_flags(2) | 0x8), 0);
    });
    let error = source
        .host_stage()
        .unwrap()
        .produce(&request(CodecKind::Hevc, 2, true))
        .err()
        .unwrap();
    app.join().unwrap();
    // The pipeline answers `EncoderFailed` with its lossless fallback (the session goes on), and the
    // next session no longer offers HEVC.
    assert_eq!(error.code, ScreenErrorCode::EncoderFailed);
    assert_eq!(channels::prereqs().refused, vec![CodecKind::Hevc]);
    assert!(matches!(
        channels::channels()[0].phase(),
        Phase::Ready { .. }
    ));
    // A failure of H.264 changes nothing in the refusals.
    channels::set_prereqs(|prereqs| prereqs.refused.clear());
    channels::refuse_codec(CodecKind::H264);
    assert!(channels::prereqs().refused.is_empty());
}
