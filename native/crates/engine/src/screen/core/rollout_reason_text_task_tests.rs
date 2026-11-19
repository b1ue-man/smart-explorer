//! Finding 3 of the measurement of 2026-11-18 (0.5.200): the host's codec
//! reasons reach a person in German; the machine-readable keys stay.
use super::*;

/// Every reason the host sends about a chosen codec reads as German, with
/// the choice named and a driver's message only behind „Meldung:“.
#[test]
fn rollout_task_codec_reasons_read_german() {
    assert_eq!(
        chosen_codec_text("chosen_codec_unavailable: av1: the host has no encoder of this codec")
            .as_deref(),
        Some("AV1 gewählt, aber das Gerät hat keinen AV1-Kodierer")
    );
    assert_eq!(
        chosen_codec_text(
            "chosen_codec_unavailable: sharp: the host has no GPU encoder of H.264 or HEVC"
        )
        .as_deref(),
        Some("Scharf (GPU) gewählt, aber das Gerät hat keinen GPU-Kodierer für H.264/HEVC")
    );
    assert_eq!(
        chosen_codec_text(
            "chosen_codec_unavailable: hevc: the viewer has no decoder of this codec"
        )
        .as_deref(),
        Some("HEVC gewählt, aber dieses Gerät hat keinen HEVC-Dekodierer")
    );
    // The reason of a failed encoder follows the text; prefixes of the
    // error's own text are skipped.
    let failed = chosen_codec_text(
        "encoder: chosen_codec_failed: av1: no encoder of the chosen codec is left: \
         no first picture from the encoder within 4000 ms",
    )
    .unwrap_or_default();
    assert_eq!(
        failed,
        "AV1 gewählt, aber alle AV1-Kodierer des Geräts sind ausgefallen: der Kodierer \
         lieferte 4000 ms lang kein erstes Bild"
    );
    let driver = chosen_codec_text(
        "chosen_codec_failed: av1: no encoder of the chosen codec is left: AMF QueryOutput: \
         result 4",
    )
    .unwrap_or_default();
    assert!(
        driver.ends_with("ausgefallen: Meldung: AMF QueryOutput: result 4"),
        "{driver}"
    );
    let bare =
        chosen_codec_text("chosen_codec_failed: hevc: no encoder of the chosen codec is left")
            .unwrap_or_default();
    assert_eq!(
        bare,
        "HEVC gewählt, aber alle HEVC-Kodierer des Geräts sind ausgefallen"
    );
    let size = chosen_codec_text(
        "chosen_codec_failed: h264: no encoder of the chosen codec is left: no pairing carries \
         7680x4320 at one frame a second",
    )
    .unwrap_or_default();
    assert!(size.ends_with("keine Kodierer-Paarung schafft 7680x4320 mit einem Bild pro Sekunde"));
    // No English word of the host's own reasons is left in these texts.
    for text in [&failed, &bare, &size] {
        for english in ["chosen_codec", "encoder", "no first picture", "the host"] {
            assert!(!text.contains(english), "{text}");
        }
    }
    assert_eq!(chosen_codec_text("device lost"), None);
    // A fallback names both codecs and stages in German.
    assert_eq!(
        fallback_detail_text("av1 hardware gave way to hevc software: busy"),
        "AV1 (Hardware) → HEVC (Software); Meldung: busy"
    );
    assert_eq!(fallback_detail_text("mf failed"), "Meldung: mf failed");
    assert_eq!(codec_key_label("tiles"), Some("Verlustfrei"));
}

/// Review 2026-11-19 (T1): the host's reasons of that review read German
/// on the desktop and in the CLI: „Scharf (GPU)“ without a GPU encoder
/// left, a refused rate decrease (Media Foundation and the GPU vendors),
/// a picture too small for the chosen codec and a viewer without a decoder
/// of its choice.
#[test]
fn review1119_task_new_reasons_read_german() {
    let sharp = chosen_codec_text(
        "chosen_codec_failed: sharp: no encoder of the chosen codec is left: Scharf (GPU): no GPU \
         encoder of hevc is left (failed: [nvenc-hevc]; software passed over: no encoder left)",
    )
    .unwrap_or_default();
    assert!(
        sharp.ends_with(
            "ausgefallen: Scharf (GPU): kein GPU-Kodierer für HEVC mehr übrig (ausgefallen: \
             [nvenc-hevc]); Software-Kodierer übergangen, Meldung: no encoder left"
        ),
        "{sharp}"
    );
    assert_eq!(
        encoder_error_text(
            "the encoder refused to lower its bit rate from 6000000 to 3000000 bit/s"
        ),
        "der Kodierer lehnte es ab, seine Bitrate von 6000000 auf 3000000 bit/s zu senken"
    );
    assert_eq!(
        encoder_error_text("OpenH264 refused to lower its bit rate from 6000000 to 3000000 bit/s"),
        "OpenH264 lehnte es ab, seine Bitrate von 6000000 auf 3000000 bit/s zu senken"
    );
    assert!(include_str!("../os/native_ffi/openh264_encode.rs")
        .contains("\"OpenH264 refused to lower its bit rate from {current} to {target} bit/s\""));
    assert_eq!(
        encoder_error_text("nvenc-h264 refused to lower its bit rate to 3000000 bit/s: status 8"),
        "nvenc-h264 lehnte es ab, seine Bitrate auf 3000000 bit/s zu senken; Meldung: status 8"
    );
    assert_eq!(
        chosen_codec_text(
            "chosen_codec_unavailable: h264: the picture of 1x1 pixels is smaller than the 2x2 \
             pixels the codec needs; the stream waits for a larger one"
        )
        .as_deref(),
        Some(
            "H.264 gewählt, das Bild mit 1x1 Pixeln ist kleiner als die 2×2 Pixel, die der \
             Codec braucht; die Übertragung wartet auf ein größeres"
        )
    );
    assert_eq!(
        chosen_codec_text(
            "encoder_failed: chosen_codec_unavailable: the viewer has no decoder of this codec"
        )
        .as_deref(),
        Some("Dieses Gerät bietet keinen Dekodierer des gewählten Codecs an")
    );
    // The host keeps writing these reasons word for word.
    let host = include_str!("../os/shared/host_worker_encoding.rs").replace("\r\n", "\n");
    assert!(host.contains("\"Scharf (GPU): no GPU encoder of {codec} is left \\\n"));
    assert!(include_str!("../os/windows/mf_rate.rs").contains(
        "\"the encoder refused to lower its bit rate from {running} to {target} bit/s\""
    ));
    assert!(include_str!("../os/windows/vendor_encode.rs")
        .contains("\"{name} refused to lower its bit rate to {} bit/s: {error}\""));
    assert!(include_str!("host_pipeline_admit.rs").contains(
        "\"chosen_codec_unavailable: {}: the picture of {}x{} pixels is smaller than \\\n"
    ));
}
