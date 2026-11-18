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
