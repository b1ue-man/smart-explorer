//! User rule of 2026-11-18 („jede Einstellung wird genau so verwendet“) in
//! the viewer session: the stored „Verlustfrei“/„Scharf (GPU)“ goes out at
//! the opening (finding 1), a new codec choice ends the viewer's own switch
//! to the tiles and a named codec never gets one (finding 2), and a session
//! that began with the tiles opens again for a video codec.
use super::super::super::screen_pool::full_request;
use super::super::super::screen_viewer_fake::{target, wait_for, Fake, DISPLAY};
use super::super::{Shared, ViewerSession};
use super::{reopen_for, tiles_allowed};
use crate::screen::core::negotiate::{CodecKind, CodecSetting};
use crate::screen::core::wire_control::{SetMode, StreamMode, ViewerFrame};
use crate::share::access_grant::CapabilityDecision;

fn full(lossless: bool) -> SetMode {
    SetMode {
        mode: StreamMode::Full,
        source: None,
        size: Some(DISPLAY),
        fps: Some(60),
        sharpness: false,
        lossless,
    }
}

#[test]
fn wahlgilt_task_opening_sends_the_stored_codec_mode() {
    let fake = Fake::new(CapabilityDecision::Allow);
    let backend = fake.backend();
    let mut request = full_request(&backend.offers, None, DISPLAY, false);
    request.codec = Some(CodecSetting::Lossless);
    let session = ViewerSession::open(&backend, target(), request, false, false, 0);
    wait_for("the connection", || fake.opens() == 1);
    fake.accept(0);
    wait_for("SET_MODE with „Verlustfrei“", || {
        fake.frames().iter().any(|frame| {
            matches!(frame, ViewerFrame::SetMode(set)
                if set.mode == StreamMode::Full && set.lossless && !set.sharpness)
        })
    });
    assert_eq!(session.shared.codec_choice(), CodecSetting::Lossless);

    // „Scharf (GPU)“ sends its bit; „Automatisch“, a named codec and a
    // preview send nothing at the opening (the hello carries them).
    let shared = Shared::new(false, 0);
    shared.set_opening(full(false), CodecSetting::Sharp);
    let mode = shared.opening_mode().expect("„Scharf (GPU)“ goes out");
    assert!(mode.sharpness && !mode.lossless);
    for codec in [CodecSetting::Auto, CodecSetting::H264, CodecSetting::Av1] {
        shared.set_opening(full(false), codec);
        assert_eq!(shared.opening_mode(), None, "{codec:?}");
    }
    let preview = SetMode {
        mode: StreamMode::Preview,
        ..full(false)
    };
    shared.set_opening(preview, CodecSetting::Lossless);
    assert_eq!(shared.opening_mode(), None);
}

#[test]
fn wahlgilt_task_new_codec_choice_ends_the_viewers_tile_switch() {
    let shared = Shared::new(false, 0);
    shared.set_opening(full(false), CodecSetting::Auto);
    let session = ViewerSession::fixture(shared.clone());
    // „Automatisch“: the viewer may switch to the tiles once.
    let switched = shared
        .request_lossless()
        .expect("the switch under „Automatisch“");
    assert!(switched.lossless);
    assert_eq!(shared.request_lossless(), None, "once");
    let mut frame = ViewerFrame::SetMode(full(false));
    shared.decoder_mode(&mut frame);
    assert!(matches!(&frame, ViewerFrame::SetMode(set) if set.lossless));
    // A new choice ends it: the next SET_MODE carries the choice alone.
    session.codec_chosen(CodecSetting::H264);
    let mut frame = ViewerFrame::SetMode(full(false));
    shared.decoder_mode(&mut frame);
    assert!(matches!(&frame, ViewerFrame::SetMode(set) if !set.lossless));
    // A named codec never gets the viewer's switch.
    assert_eq!(shared.request_lossless(), None);
    assert!(tiles_allowed(CodecSetting::Auto));
    assert!(tiles_allowed(CodecSetting::Lossless));
    for named in [
        CodecSetting::H264,
        CodecSetting::Hevc,
        CodecSetting::Av1,
        CodecSetting::Sharp,
    ] {
        assert!(!tiles_allowed(named));
    }
    // „Verlustfrei“ chose the tiles: asking for them applies the choice.
    session.codec_chosen(CodecSetting::Lossless);
    assert!(shared.request_lossless().is_some());
}

#[test]
fn wahlgilt_task_tile_session_opens_again_for_a_video_codec() {
    let tiles = Some(CodecKind::Tiles);
    assert!(reopen_for(tiles, CodecSetting::Auto));
    assert!(reopen_for(tiles, CodecSetting::Av1));
    assert!(reopen_for(tiles, CodecSetting::Sharp));
    assert!(!reopen_for(tiles, CodecSetting::Lossless));
    assert!(!reopen_for(Some(CodecKind::Hevc), CodecSetting::Av1));
    assert!(!reopen_for(None, CodecSetting::Av1));
    // The first stream counts, a later one does not.
    let shared = Shared::new(false, 0);
    shared.note_stream(CodecKind::Tiles);
    shared.note_stream(CodecKind::Av1);
    let session = ViewerSession::fixture(shared);
    assert!(session.needs_reopen(CodecSetting::H264));
    assert!(!session.needs_reopen(CodecSetting::Lossless));
    // A session opened for „Verlustfrei“ offers the tiles alone: a tile
    // session before its first stream report too (Codex 11).
    let shared = Shared::new(false, 0);
    shared.set_opening(full(false), CodecSetting::Lossless);
    let session = ViewerSession::fixture(shared);
    assert!(session.needs_reopen(CodecSetting::Av1));
    assert!(!session.needs_reopen(CodecSetting::Lossless));
    // A video session switched to „Verlustfrei“ later goes back without
    // opening again (U1).
    let shared = Shared::new(false, 0);
    shared.set_opening(full(false), CodecSetting::Auto);
    let session = ViewerSession::fixture(shared);
    session.codec_chosen(CodecSetting::Lossless);
    assert!(!session.needs_reopen(CodecSetting::H264));
}
