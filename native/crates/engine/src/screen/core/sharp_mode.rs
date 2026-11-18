//! Picture mode „Scharf (GPU)“ (plan 2026-11-17 P9, user 2026-11-17: „wir
//! brauchen noch was zwischen drinnen, schneller im idealfall um nen faktor
//! >10, die gpu nutzen wenn verfügbar zur kodierung“; „av1 ist zu viel.
//! punkt.“). It sits between the video codecs (Automatisch, H.264, HEVC,
//! AV1) and „Verlustfrei“ (the CPU tile codec):
//!
//! - the picture is encoded by a video encoder, a GPU encoder first; H.264
//!   and HEVC only, never AV1;
//! - HEVC Main 4:4:4 (wire profile `high`, B-32) first wherever host
//!   encoder and viewer decoder offer it (NVENC since Maxwell GM20x, header
//!   `NV_ENC_CAPS_SUPPORT_YUV444_ENCODE`); AMF has no 4:4:4 encode profile,
//!   so an AMD host sends 4:2:0;
//! - the encoder aims at a constant quality (`SHARP_QUALITY`, NVENC
//!   `targetQuality`, AMF `QvbrQualityLevel`) below the session's rate as its
//!   ceiling: a still desktop converges to near-visually-lossless text, a
//!   moving one loses detail instead of filling the path (the AV1 jam of
//!   2026-11-17).
//!
//! Evidence for the choice (measurement of 2026-11-17 with v0.5.198 and
//! real capture, 2880 × 1800): tiles encode 24.9 ms per picture
//! on the CPU and need 22 Mbit/s for 4.1 pictures/s; the GPU encoders encode
//! the same picture in 3–5 ms with their own device (17–18 ms through the
//! CPU convert route, which another package moves to the GPU). The
//! reference this follows is the Shadow client: FFmpeg with D3D11, H.264,
//! sharp at below 20 ms.
//!
//! This file holds the platform-free decisions: the order of the pairings
//! in this mode and the quality target. The setting key `sharp` and label
//! live with `CodecSetting` (`codec_preference.rs`).
use super::negotiate::{CodecChoice, CodecKind, CodecOffer};

/// Key of the mode in settings, IPC and `se screen view --codec`.
pub const SHARP_KEY: &str = "sharp";
/// Label in the viewer's codec list.
pub const SHARP_LABEL: &str = "Scharf (GPU)";
/// Hover text of the entry in the viewer's codec list.
pub const SHARP_HINT: &str = "Scharf: Video-Kodierer der Grafikkarte mit fester Bildqualität, \
4:4:4 wo beide Geräte es können; schneller als Verlustfrei, kein AV1.";
/// Constant-quality target of the encoders in this mode on the 1–51 scale
/// both vendors use (NVENC `NV_ENC_RC_PARAMS::targetQuality` with VBR, AMF
/// `QvbrQualityLevel` with quality VBR): 18 is the level commonly called
/// visually lossless for H.264 (x264 CRF 18); lower is sharper.
pub const SHARP_QUALITY: u8 = 18;

/// Whether `codec` may carry this mode: H.264 and HEVC, never AV1 („av1 ist
/// zu viel“) and never the tiles (they are „Verlustfrei“).
pub fn sharp_codec(codec: CodecKind) -> bool {
    matches!(codec, CodecKind::H264 | CodecKind::Hevc)
}

/// The viewer's decoder offers for this mode: H.264, HEVC and the tiles
/// (every peer's last fallback stays), without AV1.
pub fn sharp_decoders(offers: &[CodecOffer]) -> Vec<CodecOffer> {
    offers
        .iter()
        .filter(|offer| offer.codec == CodecKind::Tiles || sharp_codec(offer.codec))
        .copied()
        .collect()
}

/// The order of `choice` in this mode (smaller first): a GPU encoder before
/// a software one, a hardware decoder before a software one, 4:4:4 before
/// 4:2:0; equal ranks keep the negotiation's order (H.264 before an
/// unchecked HEVC, D6).
fn rank(choice: &CodecChoice) -> (bool, bool, bool) {
    (
        !choice.encoder_hardware,
        !choice.decoder_hardware,
        !choice.codec.chroma_444(choice.profile),
    )
}

/// The pairings of the negotiation (`negotiate_trusted`, rank order) as
/// this mode uses them: the H.264 and HEVC pairings by `rank`, then the
/// tiles as the last fallback; AV1 pairings are left out. Empty only when
/// `choices` holds neither.
pub fn sharp_order(choices: &[CodecChoice]) -> Vec<CodecChoice> {
    let mut video: Vec<CodecChoice> = choices
        .iter()
        .filter(|choice| sharp_codec(choice.codec))
        .copied()
        .collect();
    // A stable sort keeps the negotiation's order within one rank.
    video.sort_by_key(rank);
    video.extend(
        choices
            .iter()
            .filter(|choice| choice.codec == CodecKind::Tiles)
            .copied(),
    );
    video
}

/// Whether the encoder of `choice` aims at `SHARP_QUALITY` in this mode: a
/// video pairing of this mode (the tiles are lossless anyway).
pub fn quality_target(choice: &CodecChoice) -> Option<u8> {
    sharp_codec(choice.codec).then_some(SHARP_QUALITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::negotiate::{negotiate, CodecPreference, H264Profile};

    fn hardware_offers() -> Vec<CodecOffer> {
        vec![
            CodecOffer::video(CodecKind::Av1, false, true),
            CodecOffer::video(CodecKind::Hevc, true, true),
            CodecOffer::video(CodecKind::Hevc, false, true),
            CodecOffer::h264(H264Profile::High, true),
            CodecOffer::h264(H264Profile::High, false),
            CodecOffer::tiles(),
        ]
    }

    #[test]
    fn sharp_mode_task_order_prefers_gpu_444_and_never_av1() {
        let offers = hardware_offers();
        let choices = negotiate(&offers, &offers, CodecPreference::Auto);
        assert!(choices.iter().any(|choice| choice.codec == CodecKind::Av1));
        let order = sharp_order(&choices);
        assert!(order.iter().all(|choice| choice.codec != CodecKind::Av1));
        // GPU encoder and GPU decoder, 4:4:4 HEVC first.
        let first = order[0];
        assert_eq!(first.codec, CodecKind::Hevc);
        assert!(first.codec.chroma_444(first.profile));
        assert!(first.encoder_hardware && first.decoder_hardware);
        // The tiles stay the last fallback.
        assert_eq!(order.last().map(|c| c.codec), Some(CodecKind::Tiles));
        // Every GPU-encoded pairing comes before every software-encoded one.
        let last_gpu = order.iter().rposition(|c| c.encoder_hardware);
        let first_software = order
            .iter()
            .position(|c| !c.encoder_hardware && c.codec != CodecKind::Tiles);
        if let (Some(gpu), Some(software)) = (last_gpu, first_software) {
            assert!(gpu < software);
        }
    }

    #[test]
    fn sharp_mode_task_without_444_keeps_negotiated_order() {
        // An AMD host: H.264 and HEVC 4:2:0 in hardware, no 4:4:4.
        let encoders = [
            CodecOffer::video(CodecKind::Av1, false, true),
            CodecOffer::video(CodecKind::Hevc, false, true),
            CodecOffer::h264(H264Profile::High, true),
        ];
        let decoders = hardware_offers();
        let choices = negotiate(&encoders, &decoders, CodecPreference::Auto);
        let order = sharp_order(&choices);
        let codecs: Vec<CodecKind> = order
            .iter()
            .filter(|c| c.encoder_hardware && c.decoder_hardware)
            .map(|c| c.codec)
            .collect();
        // HEVC before H.264 as the negotiation ranked them (trust ALL).
        assert_eq!(codecs, [CodecKind::Hevc, CodecKind::H264]);
        assert!(order
            .iter()
            .all(|c| !c.codec.chroma_444(c.profile) && c.codec != CodecKind::Av1));
    }

    #[test]
    fn sharp_mode_task_only_av1_leaves_the_tiles() {
        let encoders = [CodecOffer::video(CodecKind::Av1, false, true)];
        let decoders = [CodecOffer::video(CodecKind::Av1, false, true)];
        let choices = negotiate(&encoders, &decoders, CodecPreference::Auto);
        let order = sharp_order(&choices);
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].codec, CodecKind::Tiles);
        assert!(sharp_order(&[]).is_empty());
    }

    #[test]
    fn sharp_mode_task_decoders_and_quality() {
        let decoders = sharp_decoders(&hardware_offers());
        assert!(decoders.iter().all(|offer| offer.codec != CodecKind::Av1));
        assert!(decoders.iter().any(|offer| offer.codec == CodecKind::Tiles));
        assert_eq!(decoders.len(), hardware_offers().len() - 1);
        let offers = hardware_offers();
        let choices = negotiate(&offers, &offers, CodecPreference::Auto);
        for choice in &choices {
            let expected = match choice.codec {
                CodecKind::H264 | CodecKind::Hevc => Some(SHARP_QUALITY),
                CodecKind::Av1 | CodecKind::Tiles => None,
            };
            assert_eq!(quality_target(choice), expected);
        }
        assert!((1..=51).contains(&SHARP_QUALITY));
        assert_eq!(SHARP_KEY, "sharp");
    }
}
