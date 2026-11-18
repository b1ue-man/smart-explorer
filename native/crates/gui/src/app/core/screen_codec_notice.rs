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
//!   text instead of sending another codec.
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
        return Some(with(text, detail));
    }
    let (rest, start) = if let Some(rest) = reason.strip_prefix(FAILED) {
        (rest, false)
    } else if let Some(rest) = reason.strip_prefix(UNAVAILABLE) {
        (rest, true)
    } else {
        return None;
    };
    // `<codec>: <reason>`, or the reason alone (a host may leave the key out).
    let (named, detail) = match rest.split_once(':') {
        Some((key, detail)) if CodecPreference::parse(key.trim()).is_some() => {
            (Some(codec_label(key.trim())), detail)
        }
        _ => (None, rest),
    };
    let named = named.or_else(|| chosen.map(|chosen| chosen.label().to_string()));
    let when = if start {
        "beim Start"
    } else {
        "in der Sitzung"
    };
    let codec = named.map_or_else(
        || "Gewählter Codec".to_string(),
        |name| format!("Gewählter Codec „{name}“"),
    );
    let text = format!(
        "{codec} ist am Gerät nicht verfügbar ({when}); es wird kein anderer gesendet, nur \
         „Automatisch“ weicht aus. Grund:"
    );
    Some(with(text, detail))
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
             mf failed"
        );
        let hevc = codec_reason_text(
            "encoder_fallback_to_hevc: av1 hardware gave way to hevc hardware: busy",
            Some(CodecSetting::Auto),
        )
        .unwrap();
        assert!(hevc.starts_with(
            "Kodierer am Gerät gewechselt: läuft jetzt HEVC (gewählt: „Automatisch“)."
        ));
        assert!(hevc.ends_with("busy"));
        for key in ["h264", "av1"] {
            let text = codec_reason_text(&format!("encoder_fallback_to_{key}: x"), None).unwrap();
            assert!(text.contains(&codec_label(key)), "{text}");
        }
        let failed = codec_reason_text(
            "chosen_codec_failed: av1: no encoder of the chosen codec is left: gone",
            None,
        )
        .unwrap();
        assert!(failed
            .starts_with("Gewählter Codec „AV1“ ist am Gerät nicht verfügbar (in der Sitzung)"));
        assert!(failed.ends_with("gone"));
        let start =
            codec_reason_text("chosen_codec_unavailable: sharp: no GPU encoder", None).unwrap();
        assert!(start.contains("beim Start"));
        assert!(start.contains(CodecSetting::Sharp.label()));
        // The reason alone: the viewer's choice names the codec.
        let bare = codec_reason_text(
            "chosen_codec_unavailable: no hardware encoder",
            Some(CodecSetting::Av1),
        )
        .unwrap();
        assert!(bare.starts_with("Gewählter Codec „AV1“ ist am Gerät nicht verfügbar (beim Start)"));
        assert!(bare.ends_with("Grund: no hardware encoder"));
        assert_eq!(codec_reason_text("pipewire_missing", None), None);
        // As the detail of `encoder_failed`.
        let detail = codec_detail_text(Some("encoder: chosen_codec_failed: hevc: no encoder"));
        assert!(detail.unwrap().contains("„HEVC“"));
        assert_eq!(codec_detail_text(Some("device lost")), None);
        assert_eq!(codec_detail_text(None), None);
    }
}
