//! The negotiation of a codec the user chose (plan 2026-11-18 „Netzrate“,
//! user rule of 2026-11-18): a chosen codec (H.264, HEVC, AV1) is always
//! the codec of the session. Another encoder of the same codec may stand in
//! (hardware, then software); another codec or the tiles never do. When no
//! pairing of the chosen codec exists the start ends with a clear error that
//! names the reason and the codec, instead of quietly sending something
//! else. „Scharf (GPU)“ stays on the GPU encoders of its codecs (H.264 and
//! HEVC), „Verlustfrei“ on the tiles; only „Automatisch“ falls back across
//! codecs (`negotiate_trusted`, unchanged).
use super::{
    negotiate_trusted, CodecChoice, CodecKind, CodecOffer, CodecPreference, CodecSetting,
    CodecTrust,
};
use crate::screen::core::frame_budget::FrameContract;
use crate::screen::core::sharp_mode;

/// The reason of a viewer that names no decoder in `SET_DECODERS`: the
/// wording of the chosen codec without a decoder, which every viewer
/// translates (`codec_reason_text`, `ScreenChoiceNotices.kt`).
/// No encoder of the host fits the viewer's decoders.
const NO_FIT: &str = "no encoder and decoder of this codec fit together";

pub const EMPTY_OFFER: &str = "the viewer has no decoder of this codec";

/// No pairing of the chosen codec: the reason for the viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChosenUnavailable {
    pub preference: CodecPreference,
    pub reason: String,
}

impl std::fmt::Display for ChosenUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A viewer that names no decoder named no choice either: the reason
        // alone, the viewer names its own choice (`EMPTY_OFFER`).
        if self.preference == CodecPreference::Auto {
            return write!(f, "chosen_codec_unavailable: {}", self.reason);
        }
        write!(
            f,
            "chosen_codec_unavailable: {}: {}",
            self.preference.key(),
            self.reason
        )
    }
}

/// The pairings a session of `preference` may use, in rank order:
/// „Automatisch“ every pairing with the tiles last, „Verlustfrei“ the tiles
/// only, a named codec its own pairings only, „Scharf (GPU)“ the H.264 and
/// HEVC pairings with a GPU encoder (`sharp_mode::sharp_order`).
pub fn negotiate_chosen(
    encoders: &[CodecOffer],
    decoders: &[CodecOffer],
    preference: CodecPreference,
    contract: Option<FrameContract>,
    trust: CodecTrust,
) -> Result<Vec<CodecChoice>, ChosenUnavailable> {
    let all = negotiate_trusted(encoders, decoders, preference, contract, trust);
    let video: Vec<CodecChoice> = match preference {
        CodecPreference::Auto | CodecPreference::Tiles if tiles_offered(decoders) => {
            return Ok(all)
        }
        // Codex 5/5: a viewer that names decoders but no tiles excluded them
        // („Dekodierer: Hardware“): no tiles are negotiated for it.
        CodecPreference::Auto | CodecPreference::Tiles => {
            let video: Vec<CodecChoice> = all.into_iter().filter(|c| c.codec.is_video()).collect();
            if video.is_empty() {
                return Err(ChosenUnavailable {
                    preference: CodecPreference::Auto,
                    reason: NO_FIT.into(),
                });
            }
            return Ok(video);
        }
        CodecPreference::Sharp => sharp_mode::sharp_order(&all)
            .into_iter()
            .filter(|choice| choice.codec.is_video() && choice.encoder_hardware)
            .collect(),
        named => all
            .into_iter()
            .filter(|choice| Some(choice.codec) == named.codec())
            .collect(),
    };
    if !video.is_empty() {
        return Ok(video);
    }
    let wanted = |offer: &CodecOffer| match preference.codec() {
        Some(codec) => offer.codec == codec,
        None => sharp_mode::sharp_codec(offer.codec) && offer.hardware,
    };
    let sharp = preference == CodecPreference::Sharp;
    let reason = if !encoders.iter().any(wanted) {
        if sharp {
            "the host has no GPU encoder of H.264 or HEVC"
        } else {
            "the host has no encoder of this codec"
        }
    } else if !decoders.iter().any(|offer| {
        preference
            .codec()
            .map_or(sharp_mode::sharp_codec(offer.codec), |c| offer.codec == c)
    }) {
        "the viewer has no decoder of this codec"
    } else {
        NO_FIT
    };
    Err(ChosenUnavailable {
        preference,
        reason: reason.into(),
    })
}

/// What the decoders of a hello or of `SET_DECODERS` say about the viewer's
/// choice (`CodecPreference::filter_decoders`): every viewer under
/// „Automatisch“ or „Verlustfrei“, and every viewer before plan 2026-11-18,
/// names the tiles; a chosen codec names its own decoders without them, and
/// „Scharf (GPU)“ those of H.264 and HEVC. Without the tiles: one video
/// codec is that codec, H.264 with HEVC is „Scharf (GPU)“ (a sharp viewer
/// with one of them only reads as that codec; `SET_MODE` `sharpness` still
/// keeps its fallbacks on the GPU, `FallbackScope`).
/// The viewer's choice: its „Codec“ as it names it (`codec_choice` of the
/// hello and of `SET_DECODERS`, review 2026-11-19 Codex 3/6) for a chosen
/// codec or „Scharf (GPU)“, else as its decoders name it
/// (`preference_named_by`). A „Scharf (GPU)“ viewer that decodes only one
/// of H.264 and HEVC read as that codec, with its software encoders; a
/// viewer before the field keeps the old reading.
/// Whether `decoders` allow the tiles: they name them, or name nothing (a
/// viewer before codec negotiation, which gets the tiles as before).
pub fn tiles_offered(decoders: &[CodecOffer]) -> bool {
    decoders.is_empty() || decoders.iter().any(|offer| offer.codec == CodecKind::Tiles)
}

pub fn preference_of(choice: Option<CodecSetting>, decoders: &[CodecOffer]) -> CodecPreference {
    // Codex 5/5: an explicit „Automatisch“ (or „Verlustfrei“, which
    // negotiates like it) is read as such; guessed from H.264 and HEVC
    // decoders under „Dekodierer: Hardware“ it read as „Scharf (GPU)“.
    match choice {
        Some(setting) => setting.preference(),
        None => preference_named_by(decoders),
    }
}

pub fn preference_named_by(decoders: &[CodecOffer]) -> CodecPreference {
    if decoders.is_empty() || decoders.iter().any(|offer| offer.codec == CodecKind::Tiles) {
        return CodecPreference::Auto;
    }
    let named = |codec: CodecKind| decoders.iter().any(|offer| offer.codec == codec);
    match (
        named(CodecKind::H264),
        named(CodecKind::Hevc),
        named(CodecKind::Av1),
    ) {
        (true, false, false) => CodecPreference::H264,
        (false, true, false) => CodecPreference::Hevc,
        (false, false, true) => CodecPreference::Av1,
        (true, true, false) => CodecPreference::Sharp,
        _ => CodecPreference::Auto,
    }
}
