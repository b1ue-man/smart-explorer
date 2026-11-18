//! Finding 2 of the measurement of 2026-11-18 (0.5.200): an encoder failure
//! reached `se screen bench` as „encoder_failed: Der Kodierer auf dem Gerät
//! ist fehlgeschlagen“ without its reason. The `ERROR` frame's detail now
//! travels with the code, the code key stays for comparisons and the text a
//! person reads names the reason in German.
use super::*;
use crate::screen::core::wire_control::ScreenWireError;
use crate::screen::core::wire_hello::ScreenErrorCode;

#[test]
fn rollout_task_encoder_error_keeps_its_reason() {
    let error = ScreenWireError::new(
        ScreenErrorCode::EncoderFailed,
        Some(
            "chosen_codec_failed: av1: no encoder of the chosen codec is left: \
             no first picture from the encoder within 4000 ms"
                .into(),
        ),
    );
    let refused = ViewerError::refused_by(&error);
    // The machine-readable key is unchanged.
    assert_eq!(refused.code(), "encoder_failed");
    // The reason reaches the person, in German.
    assert_eq!(
        refused.detail(),
        "AV1 gewählt, aber alle AV1-Kodierer des Geräts sind ausgefallen: der Kodierer \
         lieferte 4000 ms lang kein erstes Bild"
    );
    // Another reason follows the code's own text.
    let other = ViewerError::refused_by(&ScreenWireError::new(
        ScreenErrorCode::EncoderFailed,
        Some("AMF QueryOutput: result 4".into()),
    ));
    assert_eq!(
        other.detail(),
        "Der Kodierer auf dem Gerät ist fehlgeschlagen: AMF QueryOutput: result 4"
    );
    // Without a detail the code's text stays, as before.
    let bare = ViewerError::refused_by(&ScreenWireError::new(ScreenErrorCode::EncoderFailed, None));
    assert_eq!(bare.code(), "encoder_failed");
    assert_eq!(
        bare.detail(),
        "Der Kodierer auf dem Gerät ist fehlgeschlagen"
    );
    // A refusal at the start reads German as well.
    let start = ViewerError::Refused(
        "encoder_failed: chosen_codec_unavailable: av1: the host has no encoder of this codec"
            .into(),
    );
    assert_eq!(
        start.detail(),
        "AV1 gewählt, aber das Gerät hat keinen AV1-Kodierer"
    );
    // The host logs and reports every encoder failure, fallback and end
    // through one place with its reason.
    let fallback = include_str!("host_fallback.rs");
    assert!(fallback.contains("host_log::note(None, &line)"));
    let produce = include_str!("host_pipeline_produce.rs");
    assert!(!produce.contains("self.fallback_notice = Some("));
    assert!(produce.contains("self.note_encoder(format!(\"encoder_failed: {}\", failure.error))"));
    assert!(include_str!("bench.rs").contains("ViewerError::refused_by(&error)"));
}
