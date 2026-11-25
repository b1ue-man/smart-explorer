//! Settings tests of the desktop viewer (moved out of `screen_prefs.rs`,
//! which stays below 500 lines): „Codec“ and an older file's switches, the
//! output choices of P8.
use super::*;
use crate::screen::core::negotiate::CodecPreference;

/// 4.10: a stored `lossless` of an older viewer becomes „Verlustfrei“
/// (`SET_MODE` `lossless`); every choice writes `lossless` along.
#[test]
fn bildkette_task_codec_lossless_switch_is_taken_over() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("viewer.json");
    std::fs::write(&path, br#"{"schema":1,"lossless":true}"#).unwrap();
    let older = ViewerPrefs::load_from(&path);
    assert_eq!(older.codec, None);
    assert_eq!(older.codec_setting(), CodecSetting::Lossless);
    assert_eq!(older.codec_setting().preference(), CodecPreference::Auto);
    let display = crate::screen::core::pixel_convert::FrameSize::new(1920, 1080);
    let mode = super::super::screen_pool::full_set_mode(None, display, &older);
    assert!(mode.lossless, "„Verlustfrei“ still asks for the tiles");
    assert_eq!(ViewerPrefs::default().codec_setting(), CodecSetting::Auto);

    let mut chosen = older.clone();
    chosen.apply_codec((Some(CodecSetting::Hevc), Some(DecoderPreference::Software)));
    assert!(!chosen.lossless);
    chosen.save_to(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["codec"], "hevc");
    assert_eq!(stored["decoder"], "software");
    assert_eq!(stored["lossless"], false);
    assert_eq!(ViewerPrefs::load_from(&path), chosen);
    let mode = super::super::screen_pool::full_set_mode(None, display, &chosen);
    assert!(!mode.lossless);

    chosen.apply_codec((
        Some(CodecSetting::Lossless),
        Some(DecoderPreference::Automatic),
    ));
    assert!(chosen.lossless, "an older viewer reads its switch again");
    assert_eq!(chosen.codec_setting(), CodecSetting::Lossless);
}

/// P9: the switch „Schärfe vor Flüssigkeit“ is „Codec: Scharf (GPU)“,
/// read, sent as `sharpness` and written along for an older viewer.
#[test]
fn p9_task_sharpness_switch_is_the_sharp_codec() {
    let older = ViewerPrefs {
        sharpness: true,
        ..ViewerPrefs::default()
    };
    assert_eq!(older.codec_setting(), CodecSetting::Sharp);
    assert_eq!(older.codec_setting().preference(), CodecPreference::Sharp);
    let display = crate::screen::core::pixel_convert::FrameSize::new(1920, 1080);
    let mode = super::super::screen_pool::full_set_mode(None, display, &older);
    assert!(mode.sharpness && !mode.lossless);
    let mut chosen = older.clone();
    chosen.apply_codec((Some(CodecSetting::Auto), None));
    assert!(!chosen.sharpness);
    assert!(!super::super::screen_pool::full_set_mode(None, display, &chosen).sharpness);
    chosen.apply_codec((Some(CodecSetting::Sharp), None));
    assert!(chosen.sharpness && !chosen.lossless);
    assert_eq!(CodecSetting::parse("sharp"), Some(CodecSetting::Sharp));
    // Both stored switches: „Verlustfrei“ wins, as before.
    let both = ViewerPrefs {
        sharpness: true,
        lossless: true,
        ..ViewerPrefs::default()
    };
    assert_eq!(both.codec_setting(), CodecSetting::Lossless);
}

/// P8: „Dekodierer-Bibliothek“ and „Anzeige“ are stored; absent means
/// automatic, and „wgpu“ turns the presenter off.
#[test]
fn bildkette_task_ffmpeg_output_choices_are_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("viewer.json");
    std::fs::write(&path, br#"{"schema":1,"decoder":"hardware"}"#).unwrap();
    let older = ViewerPrefs::load_from(&path);
    assert_eq!(older.decoder_library, None);
    assert_eq!(older.display, None);
    assert!(DisplayChoice::default().direct());
    assert!(DisplayChoice::Direct3d11.direct());
    assert!(!DisplayChoice::Wgpu.direct());
    let mut chosen = older.clone();
    chosen.decoder_library = Some(DecoderLibrary::MediaFoundation);
    chosen.display = Some(DisplayChoice::Wgpu);
    chosen.save_to(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["decoder_library"], "media_foundation");
    assert_eq!(stored["display"], "wgpu");
    assert_eq!(stored["decoder"], "hardware");
    assert_eq!(ViewerPrefs::load_from(&path), chosen);
    for choice in DisplayChoice::ALL {
        assert_eq!(DisplayChoice::parse(choice.key()), Some(choice));
    }
}
