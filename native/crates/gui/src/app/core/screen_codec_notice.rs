//! The viewer's texts for what the host did with the chosen codec (user
//! rule of 2026-11-18: never quietly something else). The host names a
//! change of its encoder in `STATE.reason` and in error details:
//!
//! - `encoder_fallback_to_<codec>: <detail>`: under „Automatisch“ another
//!   codec (or the tiles) took over; the text names the chosen and the
//!   running codec.
//! - `chosen_codec_failed: <choice>: <detail>` (the encoder of the chosen
//!   codec failed during the session) and `chosen_codec_unavailable:
//!   <choice>: <detail>` (no pairing of it at the start): the chosen codec
//!   is not available; the session ends with the encoder error and this
//!   text instead of sending another codec. Measurement of 2026-11-18: the
//!   texts and the host's reasons in German (`core::codec_reason_text`).
use crate::screen::core::codec_reason_text::{
    chosen_text, codec_key_label, encoder_notice_text, fallback_detail_text, ENCODER_NOTICE,
};
use crate::screen::core::negotiate::{CodecPreference, CodecSetting};

const FALLBACK: &str = "encoder_fallback_to_";
const FAILED: &str = "chosen_codec_failed: ";
const UNAVAILABLE: &str = "chosen_codec_unavailable: ";

/// The name of a codec key of the host (`h264`, `tiles`, `sharp`, …).
fn codec_label(key: &str) -> String {
    let setting = match CodecPreference::parse(key) {
        Some(CodecPreference::Auto) => CodecSetting::Auto,
        Some(CodecPreference::H264) => CodecSetting::H264,
        Some(CodecPreference::Hevc) => CodecSetting::Hevc,
        Some(CodecPreference::Av1) => CodecSetting::Av1,
        Some(CodecPreference::Sharp) => CodecSetting::Sharp,
        Some(CodecPreference::Tiles) => CodecSetting::Lossless,
        None => return key.to_string(),
    };
    setting.label().to_string()
}

/// `text` with ` detail` when the host gave one.
fn with(text: String, detail: &str) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        text
    } else {
        format!("{text} {detail}")
    }
}

/// The text of a host reason about the codec; `None` for any other reason.
/// `chosen` is the viewer's „Codec“ when known.
pub fn codec_reason_text(reason: &str, chosen: Option<CodecSetting>) -> Option<String> {
    let reason = reason.trim();
    if let Some(rest) = reason.strip_prefix(FALLBACK) {
        let (key, detail) = rest.split_once(':').unwrap_or((rest, ""));
        let running = codec_label(key);
        let chosen = chosen.map_or_else(String::new, |chosen| {
            format!(" (gewählt: „{}“)", chosen.label())
        });
        let text = if key == "tiles" {
            format!("Videoencoder nicht verfügbar; verlustfreie Übertragung{chosen}.")
        } else {
            format!("Kodierer am Gerät gewechselt: läuft jetzt {running}{chosen}.")
        };
        return Some(with(text, &fallback_detail_text(detail)));
    }
    // A notice of the host's stage (`encoder_notice: …`): the encoder and the
    // codec stay, so it reads as a notice, never as a change or an error.
    if reason.starts_with(ENCODER_NOTICE.trim_end()) {
        return encoder_notice_text(reason);
    }
    let (rest, failed) = if let Some(rest) = reason.strip_prefix(FAILED) {
        (rest, true)
    } else if let Some(rest) = reason.strip_prefix(UNAVAILABLE) {
        (rest, false)
    } else {
        return None;
    };
    // `<codec>: <reason>`, or the reason alone (a host may leave the key out;
    // the viewer's choice names the codec then). Measurement of 2026-11-18:
    // the reason in German (`codec_reason_text`), never the host's English.
    let (label, detail) = match rest.split_once(':') {
        Some((key, detail)) if codec_key_label(key).is_some() => (codec_key_label(key), detail),
        _ => (chosen.map(CodecSetting::label), rest),
    };
    let text = chosen_text(label, failed, detail);
    Some(format!(
        "{text}. Es wird kein anderer Codec gesendet, nur „Automatisch“ weicht aus."
    ))
}

/// The text of an error detail that names the chosen codec (`encoder_failed`
/// and a failed session carry the host's reason as detail).
pub fn codec_detail_text(detail: Option<&str>) -> Option<String> {
    let detail = detail?.trim();
    // The reason may follow a prefix of the error's own text.
    let at = [FAILED, UNAVAILABLE, FALLBACK]
        .iter()
        .filter_map(|marker| detail.find(marker))
        .min()?;
    codec_reason_text(&detail[at..], None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding 4 (user rule 2026-11-18): every `encoder_fallback_to_<codec>`
    /// is named with chosen and running codec, and a chosen codec the host
    /// cannot keep is shown as an error with its reason.
    #[test]
    fn wahlgilt_task_codec_fallback_and_unavailable_are_named() {
        let tiles = codec_reason_text(
            "encoder_fallback_to_tiles: mf failed",
            Some(CodecSetting::Auto),
        )
        .unwrap();
        assert_eq!(
            tiles,
            "Videoencoder nicht verfügbar; verlustfreie Übertragung (gewählt: „Automatisch“). \
             Meldung: mf failed"
        );
        let hevc = codec_reason_text(
            "encoder_fallback_to_hevc: av1 hardware gave way to hevc hardware: busy",
            Some(CodecSetting::Auto),
        )
        .unwrap();
        assert!(hevc.starts_with(
            "Kodierer am Gerät gewechselt: läuft jetzt HEVC (gewählt: „Automatisch“)."
        ));
        // Measurement of 2026-11-18: the detail in German as well.
        assert!(
            hevc.ends_with("AV1 (Hardware) → HEVC (Hardware); Meldung: busy"),
            "{hevc}"
        );
        for key in ["h264", "av1"] {
            let text = codec_reason_text(&format!("encoder_fallback_to_{key}: x"), None).unwrap();
            assert!(text.contains(&codec_label(key)), "{text}");
        }
        let failed = codec_reason_text(
            "chosen_codec_failed: av1: no encoder of the chosen codec is left: gone",
            None,
        )
        .unwrap();
        assert_eq!(
            failed,
            "AV1 gewählt, aber alle AV1-Kodierer des Geräts sind ausgefallen: Meldung: gone. \
             Es wird kein anderer Codec gesendet, nur „Automatisch“ weicht aus."
        );
        let start = codec_reason_text(
            "chosen_codec_unavailable: sharp: the host has no GPU encoder of H.264 or HEVC",
            None,
        )
        .unwrap();
        assert!(start.starts_with(
            "Scharf (GPU) gewählt, aber das Gerät hat keinen GPU-Kodierer für H.264/HEVC."
        ));
        // The reason alone: the viewer's choice names the codec.
        let bare = codec_reason_text(
            "chosen_codec_unavailable: the host has no encoder of this codec",
            Some(CodecSetting::Av1),
        )
        .unwrap();
        assert!(bare.starts_with("AV1 gewählt, aber das Gerät hat keinen AV1-Kodierer."));
        assert!(!bare.contains("the host"));
        assert_eq!(codec_reason_text("pipewire_missing", None), None);
        // As the detail of `encoder_failed`.
        let detail = codec_detail_text(Some("encoder: chosen_codec_failed: hevc: no encoder"));
        assert!(detail.unwrap_or_default().starts_with("HEVC gewählt, aber"));
        assert_eq!(codec_detail_text(Some("device lost")), None);
        assert_eq!(codec_detail_text(None), None);
        // A notice of the host's stage stays a notice: no change of codec,
        // no error of the choice (fix 2026-11-19).
        let notice = codec_reason_text(
            "encoder_notice: the encoder refused the parameter quality: E_FAIL",
            Some(CodecSetting::Hevc),
        )
        .unwrap();
        assert!(
            notice.starts_with("Hinweis vom Gerät: Der Kodierer lehnte den Parameter quality ab")
        );
        assert!(!notice.contains("gewechselt") && !notice.contains("gewählt"));
        assert_eq!(codec_detail_text(Some("encoder_notice: x")), None);
    }
}
