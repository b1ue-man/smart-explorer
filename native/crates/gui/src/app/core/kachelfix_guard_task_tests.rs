//! Fix of 2026-11-18 („Kacheln immer“, v0.5.200): under „Automatisch“ a
//! codec without picture is given up for its session only, with its reason
//! in the window and the viewer log; a new session or a new codec choice
//! offers it again; a hybrid picture the decoder cannot place leaves the
//! codec, not straight to the tiles; a request without a video codec is
//! named in the viewer log.
use super::super::super::screen_pool::{preview_request, tiles_only_line};
use super::super::super::screen_session::ViewerSession;
use super::wahlgilt_guard_task_tests::state;
use super::{given_up_line, given_up_text};
use crate::screen::core::decoder_health::{GivenUp, FAILURE_LIMIT};
use crate::screen::core::negotiate::{
    CodecKind, CodecOffer, CodecSetting, DecoderPreference, H264Profile,
};
use crate::screen::core::wire_control::ViewerFrame;

#[test]
fn kachelfix_task_auto_gives_a_codec_up_for_its_session_only() {
    let (mut decode, shared, sent) = state(CodecSetting::Auto);
    for _ in 0..FAILURE_LIMIT {
        decode.guard_failure(CodecKind::Hevc);
    }
    // A host without `set_decoders_v1`: the tiles of this session.
    let frames = sent.lock().unwrap().clone();
    assert!(
        frames
            .iter()
            .any(|frame| matches!(frame, ViewerFrame::SetMode(set) if set.lossless)),
        "{frames:?}"
    );
    let notice = shared.info().notice.clone().expect("the switch is shown");
    assert!(notice.contains("Schlüsselbilder"), "{notice}");
    assert!(notice.contains("für diese Sitzung"), "{notice}");
    assert_eq!(
        shared.give_up_codec(CodecKind::Hevc),
        None,
        "given up in this session"
    );
    // Another session of the same process offers it again.
    let (_decode, other, _sent) = state(CodecSetting::Auto);
    assert!(other.give_up_codec(CodecKind::Hevc).is_some());
    // A new codec choice takes the session's give-ups back.
    let session = ViewerSession::fixture(shared.clone());
    session.codec_chosen(CodecSetting::Auto);
    assert!(shared.give_up_codec(CodecKind::Hevc).is_some());
}

#[test]
fn kachelfix_task_hybrid_failure_leaves_the_codec_once_and_names_it() {
    let reason = "kein passend zugeordnetes Hybridbild: NeedsMoreInput";
    // „Automatisch“: the session leaves the codec once.
    let (mut decode, shared, sent) = state(CodecSetting::Auto);
    assert!(decode.leave_codec(CodecKind::H264, reason));
    assert!(!decode.leave_codec(CodecKind::H264, reason), "once");
    assert_eq!(sent.lock().unwrap().len(), 1);
    let notice = shared.info().notice.clone().expect("named");
    assert!(notice.contains("Hybridbild"), "{notice}");
    // A named codec keeps its stream and shows the reason.
    let (mut decode, shared, sent) = state(CodecSetting::H264);
    assert!(!decode.leave_codec(CodecKind::H264, reason));
    assert!(sent.lock().unwrap().is_empty());
    let notice = shared.info().notice.clone().expect("named");
    assert!(notice.contains("Gewählt ist „H.264“"), "{notice}");
    assert!(notice.contains("Hybridbild"), "{notice}");
    // The log lines and texts name codec, reason and what follows.
    assert_eq!(
        given_up_line(CodecKind::H264, "r", Some(&[CodecKind::Hevc])),
        "Codec aufgegeben für diese Sitzung: h264 (r); neu verhandelt mit [hevc]"
    );
    assert!(given_up_line(CodecKind::Av1, "r", None).starts_with("Kachel-Rückfall"));
    assert!(given_up_text(CodecKind::Av1, "r", true).contains("Kachelübertragung"));
    assert!(given_up_text(CodecKind::Av1, "r", false).contains("den Codec"));
}

#[test]
fn kachelfix_task_tiles_only_request_is_named_in_the_log() {
    let mut request = preview_request(&[CodecOffer::tiles()], 320, 1000);
    request.decoders = vec![CodecOffer::tiles()];
    request.codec = Some(CodecSetting::Auto);
    request.decoder = Some(DecoderPreference::Hardware);
    let mut given_up = GivenUp::default();
    assert!(given_up.insert(CodecKind::Hevc));
    let line = tiles_only_line(&request, given_up).expect("only the tiles");
    assert!(line.starts_with("nur Kacheln angeboten"), "{line}");
    assert!(line.contains("Dekodierer „Hardware“"), "{line}");
    assert!(line.contains("aufgegeben: [hevc]"), "{line}");
    // A video codec offered: no line.
    request.decoders = vec![
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::tiles(),
    ];
    assert_eq!(tiles_only_line(&request, GivenUp::default()), None);
    // „Verlustfrei“ chose the tiles.
    request.decoders = vec![CodecOffer::tiles()];
    request.codec = Some(CodecSetting::Lossless);
    assert_eq!(tiles_only_line(&request, GivenUp::default()), None);
    // A named codec without its decoder offers nothing: named as well.
    request.codec = Some(CodecSetting::H264);
    assert!(tiles_only_line(&request, GivenUp::default()).is_some());
}
