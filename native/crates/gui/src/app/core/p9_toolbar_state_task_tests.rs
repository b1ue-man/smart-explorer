//! Plan 2026-11-17 P9: the toolbar names what the running session applies,
//! and a change stays „wird angewendet“ only until the session answered.
use std::time::{Duration, Instant};

use super::*;

fn facts(codec: CodecKind, decoder: &str, direct: bool, pictures: u64) -> SessionFacts {
    SessionFacts {
        codec: Some(codec),
        decoder: Some(decoder.to_string()),
        direct_showing: direct,
        pictures,
    }
}

const FFMPEG_HW: &str = "FFmpeg av1 [D3D11VA]";
const FFMPEG_SW: &str = "FFmpeg av1 [Software]";
const MF_HW: &str = "Microsoft AV1 Video Decoder MFT [Hardware (D3D11VA)]";
const MF_SW: &str = "Microsoft H264 Video Decoder MFT [Software (Media Foundation)]";

#[test]
fn p9_task_toolbar_names_the_applied_state() {
    // The decoder names of both libraries tell stage and library.
    assert_eq!(decoder_hardware(FFMPEG_HW), Some(true));
    assert_eq!(decoder_hardware("FFmpeg h264 [DXVA2]"), Some(true));
    assert_eq!(decoder_hardware(FFMPEG_SW), Some(false));
    assert_eq!(decoder_hardware(MF_HW), Some(true));
    assert_eq!(decoder_hardware(MF_SW), Some(false));
    assert_eq!(decoder_hardware("x [Software (OpenH264)]"), Some(false));
    assert_eq!(decoder_hardware("tiles"), None);
    assert_eq!(decoder_library(FFMPEG_SW), Some(DecoderLibrary::Ffmpeg));
    assert_eq!(
        decoder_library(MF_HW),
        Some(DecoderLibrary::MediaFoundation)
    );
    assert_eq!(
        decoder_library(MF_SW),
        Some(DecoderLibrary::MediaFoundation)
    );
    assert_eq!(decoder_library("x [Software (OpenH264)]"), None);

    let av1_sw = facts(CodecKind::Av1, FFMPEG_SW, false, 5);
    // Automatic names what it took.
    let state = codec_state(CodecSetting::Auto, &av1_sw, false);
    assert_eq!(state, Applied::Applied(Some("AV1".into())));
    assert_eq!(
        list_text("Codec", "Automatisch", &state),
        "Codec: Automatisch (AV1)"
    );
    let state = decoder_state(DecoderPreference::Automatic, &av1_sw, false);
    assert_eq!(state, Applied::Applied(Some("Software".into())));
    // A named choice the session does not run: pending, then what runs.
    assert_eq!(
        decoder_state(DecoderPreference::Hardware, &av1_sw, true),
        Applied::Pending
    );
    let other = decoder_state(DecoderPreference::Hardware, &av1_sw, false);
    assert_eq!(other, Applied::Other("Software".into()));
    assert_eq!(
        list_text("Dekodierer", "Hardware", &other),
        "Dekodierer: Hardware → läuft: Software"
    );
    assert_eq!(
        list_text("Codec", "H.264", &Applied::Pending),
        "Codec: H.264 (wird angewendet …)"
    );
    assert_eq!(
        codec_state(CodecSetting::H264, &av1_sw, false),
        Applied::Other("AV1".into())
    );
    assert_eq!(
        codec_state(CodecSetting::Av1, &av1_sw, true),
        Applied::Applied(None)
    );
    // „Verlustfrei“ runs as the tiles; a named decoder does not apply to them
    // (user rule 2026-11-18), „Automatisch“ names them.
    let tiles = facts(CodecKind::Tiles, "tiles", false, 5);
    assert_eq!(
        codec_state(CodecSetting::Lossless, &tiles, false),
        Applied::Applied(None)
    );
    assert_eq!(
        codec_state(CodecSetting::Auto, &tiles, false),
        Applied::Other("Verlustfrei".into())
    );
    assert_eq!(
        decoder_state(DecoderPreference::Hardware, &tiles, false),
        Applied::Other("Kacheln".into())
    );
    // Library.
    let mf = facts(CodecKind::Av1, MF_HW, false, 5);
    assert_eq!(
        library_state(DecoderLibrary::Ffmpeg, &mf, false),
        Applied::Other("Media Foundation".into())
    );
    assert_eq!(
        library_state(DecoderLibrary::Automatic, &mf, false),
        Applied::Applied(Some("Media Foundation".into()))
    );
    // Display: a picture „Direct3D 11“ does not show names the way it takes
    // and why (user rule 2026-11-18).
    let display =
        |chosen, facts: &SessionFacts| display_state_in(chosen, facts, false, "wgpu", None);
    let d3d11 = facts(CodecKind::Av1, FFMPEG_HW, false, 5);
    assert_eq!(
        display(DisplayChoice::Direct3d11, &d3d11),
        Applied::Other("kein Bild – Direct3D 11 zeigt das Bild noch nicht".into())
    );
    assert_eq!(
        display(DisplayChoice::Direct3d11, &mf),
        Applied::Other(format!(
            "kein Bild – Bild ist keine D3D11VA-Textur ({MF_HW})"
        ))
    );
    let shown = facts(CodecKind::Av1, FFMPEG_HW, true, 5);
    assert_eq!(
        display(DisplayChoice::Wgpu, &shown),
        Applied::Other("Direct3D 11".into())
    );
    assert_eq!(
        display(DisplayChoice::Automatic, &shown),
        Applied::Applied(Some("Direct3D 11".into()))
    );
    // Before the first facts nothing is claimed.
    let none = SessionFacts::default();
    assert_eq!(
        codec_state(CodecSetting::H264, &none, true),
        Applied::Unknown
    );
    assert_eq!(
        display_state_in(DisplayChoice::Wgpu, &none, true, "wgpu", None),
        Applied::Unknown
    );
}

#[test]
fn p9_task_toolbar_change_pending_until_the_session_answers() {
    let start = Instant::now();
    let before = facts(CodecKind::Av1, FFMPEG_SW, false, 10);
    let mut pending = PendingChoices::default();
    pending.start(Setting::Decoder, &before, start);
    assert!(pending.pending(Setting::Decoder, &before, start));
    assert!(!pending.pending(Setting::Codec, &before, start));
    // Pictures go on with unchanged facts: still waiting inside the window.
    let later = SessionFacts {
        pictures: 40,
        ..before.clone()
    };
    let inside = start + ANSWER_WINDOW - Duration::from_millis(1);
    assert!(pending.pending(Setting::Decoder, &later, inside));
    // After the window with new pictures the change counts as answered.
    assert!(!pending.pending(Setting::Decoder, &later, start + ANSWER_WINDOW));
    assert!(!pending.pending(Setting::Decoder, &later, start + ANSWER_WINDOW));
    // No pictures at all (a stalled stream): it keeps waiting.
    pending.start(Setting::Codec, &before, start);
    let stalled = start + ANSWER_WINDOW * 10;
    assert!(pending.pending(Setting::Codec, &before, stalled));
    // Changed facts end the wait at once.
    let switched = facts(CodecKind::Av1, FFMPEG_HW, false, 11);
    assert!(!pending.pending(Setting::Codec, &switched, start));
    // A new change of the same setting replaces the old one.
    pending.start(Setting::Display, &before, start);
    pending.start(Setting::Display, &switched, start);
    assert!(pending.pending(Setting::Display, &switched, start));
}

#[test]
fn p9_task_toolbar_cuts_long_list_texts() {
    assert_eq!(short_text("Ganzer Bildschirm"), "Ganzer Bildschirm");
    let title = "ä".repeat(SHORT_TEXT + 10);
    let cut = short_text(&title);
    assert_eq!(cut.chars().count(), SHORT_TEXT);
    assert!(cut.ends_with('…'));
    assert_eq!(short_text(&"x".repeat(SHORT_TEXT)), "x".repeat(SHORT_TEXT));
}

/// Findings 5 and 7 of the user rule of 2026-11-18: no list says
/// „angewendet“ for a choice the session does not run.
#[test]
fn wahlgilt_task_toolbar_never_claims_an_unapplied_choice() {
    let tiles = facts(CodecKind::Tiles, "tiles", false, 5);
    for named in [DecoderPreference::Hardware, DecoderPreference::Software] {
        assert_eq!(
            decoder_state(named, &tiles, false),
            Applied::Other("Kacheln".into())
        );
    }
    assert_eq!(
        decoder_state(DecoderPreference::Automatic, &tiles, false),
        Applied::Applied(Some("Kacheln".into()))
    );
    assert_eq!(
        library_state(DecoderLibrary::MediaFoundation, &tiles, false),
        Applied::Other("Kacheln".into())
    );
    // „Direct3D 11“ chosen, the presenter failed: its error is named.
    let d3d11 = facts(CodecKind::Hevc, "FFmpeg hevc [D3D11VA]", false, 5);
    assert_eq!(
        display_state_in(
            DisplayChoice::Direct3d11,
            &d3d11,
            false,
            "wgpu",
            Some("no device")
        ),
        Applied::Other("kein Bild – Direct3D 11 nicht verfügbar: no device".into())
    );
    // „wgpu“ chosen while glow draws the window.
    let shown = facts(CodecKind::Hevc, MF_HW, false, 5);
    assert_eq!(
        display_state_in(DisplayChoice::Wgpu, &shown, false, "OpenGL (glow)", None),
        Applied::Other("OpenGL (glow)".into())
    );
    assert_eq!(
        display_state_in(DisplayChoice::Wgpu, &shown, false, "wgpu", None),
        Applied::Applied(None)
    );
    assert_eq!(
        display_state_in(
            DisplayChoice::Automatic,
            &shown,
            false,
            "OpenGL (glow)",
            None
        ),
        Applied::Applied(Some("OpenGL (glow)".into()))
    );
    // Tiles are no D3D11VA texture either.
    assert_eq!(
        display_state_in(DisplayChoice::Direct3d11, &tiles, false, "wgpu", None),
        Applied::Other("kein Bild – Bild ist keine D3D11VA-Textur (tiles)".into())
    );
}
