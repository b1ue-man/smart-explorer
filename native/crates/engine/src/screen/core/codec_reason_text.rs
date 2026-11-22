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
const SHARP_LEFT: &str = "Scharf (GPU): no GPU encoder of ";
/// `<name> refused to lower its bit rate from <rate> to <rate> bit/s`
/// (Media Foundation names itself „the encoder“, OpenH264 by its name).
const REFUSED_LOWER_FROM: &str = " refused to lower its bit rate from ";
const VENDOR_REFUSED_LOWER: &str = " refused to lower its bit rate to ";
const SMALL_PICTURE: &str = "the picture of ";
/// The prefix of a stage's notice that ends nothing (`HostStage::
/// take_notice`, review 2026-11-19): `encoder_notice: <sentence>`.
pub const ENCODER_NOTICE: &str = "encoder_notice: ";
/// The sentences a stage's notice uses (`HostStage::take_notice`).
const NOTICE_LOWEST: &str = "the encoder's lowest bit rate ";
const NOTICE_ABOVE: &str = " bit/s is above the highest data rate ";
const NOTICE_PARAMETER: &str = "the encoder refused the parameter ";
const NOTICE_CONTROL: &str = "the control stream's ";
const NOTICE_CONTROL_ABOVE: &str = " bit/s leave the pictures too little of the highest data rate ";
const NOTICE_SOUND: &str = "the sound and the pictures' least share need ";
const NOTICE_SOUND_ABOVE: &str = " bit/s, more than the highest data rate ";

/// The notice of sound that does not fit under „Höchste Datenrate“ (round 8
/// Gemini 8/10): no sound is sent (`audio_codec::sound_fits`).
pub fn sound_over_ceiling(needed_bps: u64, ceiling_bps: u64) -> String {
    format!("{ENCODER_NOTICE}{NOTICE_SOUND}{needed_bps}{NOTICE_SOUND_ABOVE}{ceiling_bps} bit/s; no sound is sent")
}
/// An input the Android host's accessibility service did not perform
/// (`screen.hostNotice {inputRefused}`, `mobile/…/screen_host.rs`) and its
/// German text.
const NOTICE_INPUT: [(&str, &str); 7] = [
    (
        "the device cancelled a gesture of the input",
        "Das Gerät hat eine Geste der Eingabe abgebrochen",
    ),
    (
        "the device refused a text of the input (no focused field)",
        "Das Gerät hat einen Text der Eingabe abgelehnt (kein Eingabefeld im Fokus)",
    ),
    (
        "the device refused a system action of the input",
        "Das Gerät hat eine Systemaktion der Eingabe abgelehnt",
    ),
    (
        "the device did not answer an input in time",
        "Das Gerät hat auf eine Eingabe nicht rechtzeitig geantwortet",
    ),
    (
        "the device has no counterpart for this mouse button (only left and back)",
        "Das Gerät kennt diese Maustaste nicht (nur links und „Zurück“)",
    ),
    (
        "the device cannot see the content of this field (a password or web field); typing would \
         replace it, so the text was not typed",
        "Das Gerät kann den Inhalt dieses Felds nicht sehen (Passwort- oder Webfeld); Tippen würde ihn \
         ersetzen, der Text wurde daher nicht eingegeben",
    ),
    (
        "the focused field did not take the text (it is read-only or refuses text)",
        "Das Feld mit dem Fokus hat den Text nicht angenommen (schreibgeschützt oder es nimmt keinen Text an)",
    ),
];

/// The German text of a stage's notice anywhere in `reason`
/// (`encoder_notice: <sentence>`); `None` when `reason` holds none. The
/// encoder keeps running: the viewer shows a notice, no error.
pub fn encoder_notice_text(reason: &str) -> Option<String> {
    encoder_notice_german(reason)
}

/// The host's sentence when `reason` (a `STATE` reason) is the notice of an
/// input the device did not perform (an Android host's `inputRefused`: a
/// mouse button without counterpart, a hidden field, a refused text or
/// action); `None` for every other reason. `se screen input` and MCP report
/// it as not applied (round 11, M3).
pub fn input_refusal(reason: &str) -> Option<&'static str> {
    let at = reason.find(ENCODER_NOTICE)?;
    let text = reason[at + ENCODER_NOTICE.len()..].trim();
    NOTICE_INPUT
        .iter()
        .find(|(english, _)| text == *english)
        .map(|(english, _)| *english)
}

fn encoder_notice_german(reason: &str) -> Option<String> {
    let at = reason.find(ENCODER_NOTICE)?;
    let text = reason[at + ENCODER_NOTICE.len()..].trim();
    Some(format!("Hinweis vom Gerät: {}", notice_sentence(text)))
}

/// `the encoder's lowest bit rate <n> bit/s is above the highest data rate
/// <n> bit/s; fewer pictures keep the ceiling` and `the encoder refused the
/// parameter <name>: <detail>` in German; another sentence as the host's
/// reasons are (`encoder_error_text`).
fn notice_sentence(text: &str) -> String {
    if let Some((lowest, ceiling)) = text
        .strip_prefix(NOTICE_LOWEST)
        .and_then(|rest| rest.split_once(NOTICE_ABOVE))
    {
        let ceiling = ceiling.split_once(" bit/s").map_or(ceiling, |(n, _)| n);
        return format!(
            "Die niedrigste Bitrate des Kodierers ({lowest} bit/s) liegt über der Höchsten \
             Datenrate ({ceiling} bit/s); weniger Bilder halten die Obergrenze ein"
        );
    }
    // The Android bridge knows only the rate it asked for, not the ceiling.
    if let Some((lowest, asked)) = text
        .strip_prefix(NOTICE_LOWEST)
        .and_then(|rest| rest.split_once(" bit/s is above the bit rate "))
    {
        let asked = asked.split_once(" bit/s").map_or(asked, |(n, _)| n);
        return format!(
            "Die niedrigste Bitrate des Kodierers ({lowest} bit/s) liegt über der verlangten \
             Rate ({asked} bit/s); er sendet mit seiner niedrigsten Rate, die Höchste Datenrate \
             hält das Datenbudget ein"
        );
    }
    if let Some((_, german)) = NOTICE_INPUT.iter().find(|(english, _)| text == *english) {
        return (*german).to_string();
    }
    if let Some((control, ceiling)) = text
        .strip_prefix(NOTICE_CONTROL)
        .and_then(|rest| rest.split_once(NOTICE_CONTROL_ABOVE))
    {
        let ceiling = ceiling.trim_end_matches(" bit/s");
        return format!(
            "Steuerkanal und Ton (Zeiger, Status, Fensterlisten, Bildschirmton) brauchen {control} bit/s \
             der Höchsten Datenrate ({ceiling} bit/s); für Bilder bleibt zu wenig, neue Bilder \
             werden zurückgehalten, bis sie weniger brauchen"
        );
    }
    if let Some((needed, ceiling)) = text
        .strip_prefix(NOTICE_SOUND)
        .and_then(|rest| rest.split_once(NOTICE_SOUND_ABOVE))
    {
        let ceiling = ceiling.split_once(" bit/s").map_or(ceiling, |(n, _)| n);
        return format!(
            "Ton passt neben dem Bild nicht unter die Höchste Datenrate: Ton und Mindestanteil \
             der Bilder brauchen {needed} bit/s, die Höchste Datenrate ist {ceiling} bit/s; es \
             wird kein Ton gesendet"
        );
    }
    if let Some(rest) = text.strip_prefix(NOTICE_PARAMETER) {
        let (name, detail) = rest.split_once(": ").unwrap_or((rest, ""));
        let base = format!("Der Kodierer lehnte den Parameter {name} ab");
        return if detail.is_empty() {
            base
        } else {
            format!("{base}; Meldung: {detail}")
        };
    }
    encoder_error_text(text)
}

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
    if let Some(text) = review_error_text(error) {
        return text;
    }
    if error.is_empty() {
        return String::new();
    }
    format!("Meldung: {error}")
}

/// The reasons the host names since the review of 2026-11-19 in German:
/// „Scharf (GPU)“ without a GPU encoder left (`host_worker_encoding.rs`),
/// an encoder that refused to lower its rate (`mf_rate.rs`,
/// `vendor_encode.rs`), a picture too small for the chosen codec
/// (`host_pipeline_admit.rs`) and a viewer without a decoder of its choice
/// (`negotiate::EMPTY_OFFER`).
fn review_error_text(error: &str) -> Option<String> {
    if let Some(rest) = error.strip_prefix(SHARP_LEFT) {
        let (codec, rest) = rest.split_once(" is left").unwrap_or((rest, ""));
        let codec = codec_key_label(codec).unwrap_or(codec);
        let inner = rest.trim();
        let inner = inner.strip_prefix('(').unwrap_or(inner);
        let inner = inner.strip_suffix(')').unwrap_or(inner);
        let (failed, passed) = inner
            .split_once("; software passed over: ")
            .unwrap_or((inner, ""));
        let failed = failed.trim_start_matches("failed: ").trim();
        let mut text = format!("Scharf (GPU): kein GPU-Kodierer für {codec} mehr übrig");
        if !failed.is_empty() && failed != "[]" {
            text.push_str(&format!(" (ausgefallen: {failed})"));
        }
        if !passed.is_empty() {
            text.push_str(&format!(
                "; Software-Kodierer übergangen, Meldung: {passed}"
            ));
        }
        return Some(text);
    }
    if let Some((name, rest)) = error.split_once(REFUSED_LOWER_FROM) {
        let (from, to) = rest.trim_end_matches(" bit/s").split_once(" to ")?;
        let name = if name == "the encoder" {
            "der Kodierer"
        } else {
            name
        };
        return Some(format!(
            "{name} lehnte es ab, seine Bitrate von {from} auf {to} bit/s zu senken"
        ));
    }
    if let Some((name, rest)) = error.split_once(VENDOR_REFUSED_LOWER) {
        let (rate, message) = rest.split_once(" bit/s").unwrap_or((rest, ""));
        let message = message.trim_start_matches(':').trim();
        let mut text = format!("{name} lehnte es ab, seine Bitrate auf {rate} bit/s zu senken");
        if !message.is_empty() {
            text.push_str(&format!("; Meldung: {message}"));
        }
        return Some(text);
    }
    if let Some(rest) = error.strip_prefix(SMALL_PICTURE) {
        let size = rest.split_whitespace().next().unwrap_or(rest);
        return Some(format!(
            "das Bild mit {size} Pixeln ist kleiner als die 2×2 Pixel, die der Codec braucht; \
             die Übertragung wartet auf ein größeres"
        ));
    }
    (error == super::negotiate::EMPTY_OFFER)
        .then(|| "dieses Gerät bietet keinen Dekodierer des gewählten Codecs an".into())
}

/// The text of a chosen codec the host cannot keep: `label` is the choice
/// („AV1“, „Scharf (GPU)“; `None` when the host named none), `failed` the
/// end in the session (else the start), `reason` the host's reason.
pub fn chosen_text(label: Option<&str>, failed: bool, reason: &str) -> String {
    let reason = reason.trim();
    let Some(label) = label else {
        // A viewer that named no decoder (`EMPTY_OFFER`) names its choice
        // itself; without it the reason alone (review 2026-11-19).
        if reason == super::negotiate::EMPTY_OFFER {
            return "Dieses Gerät bietet keinen Dekodierer des gewählten Codecs an".into();
        }
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
    if reason.starts_with(SMALL_PICTURE) {
        // The chosen codec waits for a larger picture (review 2026-11-19).
        return format!("{label} gewählt, {}", encoder_error_text(reason));
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
        super::set_decoders::TILES_SESSION => format!(
            "{label} gewählt, aber diese Sitzung wurde für „Verlustfrei“ geöffnet; sie muss für \
             diesen Codec neu geöffnet werden"
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
