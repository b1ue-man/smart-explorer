//! The host's codec reasons in German for every surface (measurement of
//! 2026-11-18 with 0.5.200: „Der Kodierer auf dem Gerät ist fehlgeschlagen:
//! chosen_codec_unavailable: av1: the host has no encoder of this codec“
//! mixed German and English). The host keeps sending its machine-readable
//! reasons (`chosen_codec_unavailable: <choice>: <reason>`,
//! `chosen_codec_failed: <choice>: no encoder of the chosen codec is left:
//! <error>`, `encoder_fallback_to_<codec>: <failed> <stage> gave way to
//! <next> <stage>: <error>`); the code field of an error stays the key. Only
//! the text a person reads is German: the viewer's error
//! (`viewer_client::ViewerError::detail`, CLI), the desktop notices
//! (`gui/app/core/screen_codec_notice.rs`) and Android (`ScreenChoiceNotices.kt`,
//! its own copy of these texts). A driver's own message cannot be translated;
//! it follows as „Meldung: …“.
use super::negotiate::{CodecPreference, CodecSetting};

const FAILED: &str = "chosen_codec_failed: ";
const UNAVAILABLE: &str = "chosen_codec_unavailable: ";
const LEFT: &str = "no encoder of the chosen codec is left";
const FIRST_PICTURE: &str = "no first picture from the encoder within ";
const NO_PAIRING: &str = "no pairing carries ";

/// The label of a codec or choice key of the host (`av1`, `sharp`, …).
pub fn codec_key_label(key: &str) -> Option<&'static str> {
    let setting = match CodecPreference::parse(key.trim())? {
        CodecPreference::Auto => CodecSetting::Auto,
        CodecPreference::H264 => CodecSetting::H264,
        CodecPreference::Hevc => CodecSetting::Hevc,
        CodecPreference::Av1 => CodecSetting::Av1,
        CodecPreference::Sharp => CodecSetting::Sharp,
        CodecPreference::Tiles => CodecSetting::Lossless,
    };
    Some(setting.label())
}

/// „Scharf (GPU)“ names the GPU encoders of H.264 and HEVC, a codec its own.
fn encoders_of(label: &str) -> String {
    if label == super::sharp_mode::SHARP_LABEL {
        "GPU-Kodierer für H.264/HEVC".into()
    } else {
        format!("{label}-Kodierer")
    }
}

fn decoders_of(label: &str) -> String {
    if label == super::sharp_mode::SHARP_LABEL {
        "Dekodierer für H.264/HEVC".into()
    } else {
        format!("{label}-Dekodierer")
    }
}

/// An encoder error of the host in German; a text the host does not name
/// itself (a driver's message) follows as „Meldung: …“.
pub fn encoder_error_text(error: &str) -> String {
    let error = error.trim().trim_start_matches(':').trim();
    if let Some(rest) = error.strip_prefix(FIRST_PICTURE) {
        let millis = rest.trim_end_matches(" ms").trim();
        return format!("der Kodierer lieferte {millis} ms lang kein erstes Bild");
    }
    if let Some(rest) = error.strip_prefix(NO_PAIRING) {
        let size = rest.split_whitespace().next().unwrap_or(rest);
        return format!("keine Kodierer-Paarung schafft {size} mit einem Bild pro Sekunde");
    }
    if error.is_empty() {
        return String::new();
    }
    format!("Meldung: {error}")
}

/// The text of a chosen codec the host cannot keep: `label` is the choice
/// („AV1“, „Scharf (GPU)“; `None` when the host named none), `failed` the
/// end in the session (else the start), `reason` the host's reason.
pub fn chosen_text(label: Option<&str>, failed: bool, reason: &str) -> String {
    let reason = reason.trim();
    let Some(label) = label else {
        let why = encoder_error_text(reason.trim_start_matches(LEFT));
        let base = if failed {
            "Alle Kodierer des gewählten Codecs sind am Gerät ausgefallen"
        } else {
            "Der gewählte Codec ist am Gerät nicht verfügbar"
        };
        return with_reason(base.into(), &why);
    };
    if failed {
        let why = encoder_error_text(reason.strip_prefix(LEFT).unwrap_or(reason));
        let base = format!(
            "{label} gewählt, aber alle {} des Geräts sind ausgefallen",
            encoders_of(label)
        );
        return with_reason(base, &why);
    }
    match reason {
        "the host has no encoder of this codec" | "the host has no GPU encoder of H.264 or HEVC" => {
            format!(
                "{label} gewählt, aber das Gerät hat keinen {}",
                encoders_of(label)
            )
        }
        "the viewer has no decoder of this codec" => format!(
            "{label} gewählt, aber dieses Gerät hat keinen {}",
            decoders_of(label)
        ),
        "no encoder and decoder of this codec fit together" => format!(
            "{label} gewählt, aber die {} des Geräts und die {} dieses Geräts passen nicht zusammen",
            encoders_of(label),
            decoders_of(label)
        ),
        other => with_reason(
            format!("{label} gewählt, aber am Gerät nicht verfügbar"),
            &encoder_error_text(other),
        ),
    }
}

fn with_reason(base: String, why: &str) -> String {
    if why.is_empty() {
        base
    } else {
        format!("{base}: {why}")
    }
}

/// The German text of a `chosen_codec_failed` or `chosen_codec_unavailable`
/// reason anywhere in `text` (an error detail may carry a prefix); `None`
/// when `text` names neither.
pub fn chosen_codec_text(text: &str) -> Option<String> {
    let (at, failed, marker) = [(FAILED, true), (UNAVAILABLE, false)]
        .into_iter()
        .filter_map(|(marker, failed)| text.find(marker).map(|at| (at, failed, marker)))
        .min_by_key(|(at, _, _)| *at)?;
    let rest = &text[at + marker.len()..];
    let (label, reason) = match rest.split_once(':') {
        Some((key, reason)) => match codec_key_label(key) {
            Some(label) => (Some(label), reason),
            None => (None, rest),
        },
        None => (None, rest),
    };
    Some(chosen_text(label, failed, reason))
}

/// „Hardware“ or „Software“ for the stage word of the host.
fn stage_label(stage: &str) -> &str {
    match stage {
        "hardware" => "Hardware",
        "software" => "Software",
        other => other,
    }
}

/// The German detail of `encoder_fallback_to_<codec>: <detail>`:
/// `<failed> <stage> gave way to <next> <stage>: <error>` becomes
/// „AV1 (Hardware) → HEVC (Hardware); Meldung: …“; another detail (the
/// tiles' fallback carries the error alone) is `encoder_error_text`.
pub fn fallback_detail_text(detail: &str) -> String {
    let detail = detail.trim();
    let Some((pairs, error)) = detail.split_once(" gave way to ").map(|(failed, rest)| {
        let (next, error) = rest.split_once(':').unwrap_or((rest, ""));
        ((failed, next), error)
    }) else {
        return encoder_error_text(detail);
    };
    let name = |text: &str| -> String {
        let mut words = text.split_whitespace();
        let codec = words.next().unwrap_or_default();
        let label = codec_key_label(codec).unwrap_or(codec);
        match words.next() {
            Some(stage) => format!("{label} ({})", stage_label(stage)),
            None => label.to_string(),
        }
    };
    let text = format!("{} → {}", name(pairs.0), name(pairs.1));
    let why = encoder_error_text(error);
    if why.is_empty() {
        text
    } else {
        format!("{text}; {why}")
    }
}

#[cfg(test)]
#[path = "rollout_reason_text_task_tests.rs"]
mod rollout_reason_text_task_tests;
