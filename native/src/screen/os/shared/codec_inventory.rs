//! The codecs of this process (SCR-M3, decision C2 „Kodierer“ and
//! „Dekodierer“): on Windows the Media Foundation transforms found at run
//! time (`MFStartup` and `MFTEnumEx`; without an H.264 encoder there is no
//! Media Foundation entry, nothing is derived from the edition), hardware
//! before software; on Windows and Linux OpenH264 when the user switched it
//! on; the lossless tiles always last (`CodecRegistry`). Android has no
//! H.264 before SCR-M8.
//!
//! OpenH264 encodes Constrained Baseline up to 3840 × 2160. Its decoder
//! reports `cb` and `high`, because a host never sends B frames, so its
//! `high` is Constrained High (assumption A4, checked by
//! `screen_task_openh264_decodes_constrained_high`). Should that check fail,
//! `OPENH264_DECODES_HIGH` becomes `false` and OpenH264 viewers report `cb`
//! only.
use std::path::Path;

use crate::screen::core::codec_registry::{CodecRegistry, CodecReport, OpenH264Report};
use crate::screen::core::negotiate::{CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

/// Assumption A4: OpenH264 decodes High without B frames (Constrained High).
pub(crate) const OPENH264_DECODES_HIGH: bool = true;
/// Largest frame of the OpenH264 encoder (`openh264` 0.9.9 refuses more).
pub(crate) const OPENH264_MAX_ENCODE: FrameSize = FrameSize::new(3840, 2160);

/// What the OpenH264 encoder offers.
pub(crate) fn openh264_encoder_offers() -> Vec<CodecOffer> {
    vec![CodecOffer::h264(H264Profile::ConstrainedBaseline, false).with_max(OPENH264_MAX_ENCODE)]
}

/// What the OpenH264 decoder offers (rule A4 above), up to level 5.2.
pub(crate) fn openh264_decoder_offers() -> Vec<CodecOffer> {
    let profiles: &[H264Profile] = if OPENH264_DECODES_HIGH {
        &[H264Profile::High, H264Profile::ConstrainedBaseline]
    } else {
        &[H264Profile::ConstrainedBaseline]
    };
    profiles
        .iter()
        .map(|profile| CodecOffer::h264(*profile, false).with_max(FrameSize::MAX))
        .collect()
}

/// The codecs of this process and the state of OpenH264.
pub(crate) struct Inventory {
    pub registry: CodecRegistry,
    pub openh264: OpenH264Report,
}

/// The codecs with the OpenH264 store `openh264_dir` (`None`: this user's).
/// `verify` checks the OpenH264 file against its pin for the report; without
/// it the check runs before every load.
pub(crate) fn inventory(openh264_dir: Option<&Path>, verify: bool) -> Inventory {
    let (mut encoders, mut decoders) = super::media_foundation_codecs();
    let (openh264_encoders, openh264_decoders, openh264) =
        super::openh264_codecs(openh264_dir, verify);
    encoders.extend(openh264_encoders);
    decoders.extend(openh264_decoders);
    Inventory {
        registry: CodecRegistry::new(encoders, decoders),
        openh264,
    }
}

/// The codecs of this process and this user.
pub(crate) fn registry() -> CodecRegistry {
    inventory(None, false).registry
}

/// `se screen codec --json` of this process and this user.
pub(crate) fn report() -> CodecReport {
    let inventory = inventory(None, true);
    inventory.registry.report(inventory.openh264)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::codec_registry::{CodecEntry, OpenH264State};
    use crate::screen::core::negotiate::{negotiate, CodecChoice, CodecKind, CodecPreference};

    fn tiles_entry() -> CodecEntry {
        CodecEntry {
            codec: CodecKind::Tiles,
            profiles: Vec::new(),
            name: "tiles".into(),
            hardware: false,
        }
    }

    #[test]
    fn screen_task_codec_inventory_without_mf() {
        // A store without OpenH264: Media Foundation where this host has it,
        // the tiles always. The GitHub Windows runner has no Media
        // Foundation; a build that bound `mfplat.dll` at load time would not
        // even start there.
        let empty = tempfile::tempdir().unwrap();
        let found = inventory(Some(empty.path()), true);
        let report = found.registry.report(found.openh264.clone());
        assert_eq!(report.encoders.last(), Some(&tiles_entry()));
        assert_eq!(report.decoders.last(), Some(&tiles_entry()));
        let others = report.encoders.len() - 1;
        if super::super::media_foundation_present() {
            assert!(
                others > 0,
                "Media Foundation without an encoder row: {report:?}"
            );
            assert!(report.encoders.iter().all(|entry| entry.name != "openh264"));
        } else {
            assert_eq!(report.encoders, vec![tiles_entry()]);
            assert_eq!(report.decoders, vec![tiles_entry()]);
            assert_eq!(found.registry.encoder_offers(), vec![CodecOffer::tiles()]);
            assert_eq!(found.registry.decoder_offers(), vec![CodecOffer::tiles()]);
        }
        assert_eq!(report.openh264.state, OpenH264State::Absent);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["openh264"]["state"], "absent");
        assert_eq!(json["openh264"]["version"], "2.6.0");
        assert_eq!(
            json["openh264"]["notice"],
            "OpenH264 Video Codec provided by Cisco Systems, Inc."
        );
        let last = json["encoders"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            last,
            serde_json::json!({"codec": "tiles", "profiles": [], "name": "tiles", "hardware": false})
        );
        // A host without H.264 still negotiates the tiles with every viewer.
        let choices = negotiate(
            &found.registry.encoder_offers(),
            &openh264_decoder_offers(),
            CodecPreference::Auto,
        );
        assert_eq!(
            choices.last().map(|choice| choice.codec),
            Some(CodecKind::Tiles)
        );
    }

    #[test]
    fn screen_task_negotiate_cb_high_for_openh264_viewer() {
        let viewer = openh264_decoder_offers();
        let cb = CodecOffer::h264(H264Profile::ConstrainedBaseline, false).with_max(FrameSize::MAX);
        let high = CodecOffer::h264(H264Profile::High, false).with_max(FrameSize::MAX);
        assert!(viewer.contains(&cb));
        assert_eq!(viewer.contains(&high), OPENH264_DECODES_HIGH);
        let json = serde_json::to_value(&viewer).unwrap();
        assert_eq!(json[viewer.len() - 1]["profile"], "cb");
        // A Windows host: hardware MFT with both profiles, the software MFT,
        // OpenH264 and the tiles.
        let mut windows = vec![
            CodecOffer::h264(H264Profile::High, true),
            CodecOffer::h264(H264Profile::ConstrainedBaseline, true),
            CodecOffer::h264(H264Profile::High, false),
            CodecOffer::h264(H264Profile::ConstrainedBaseline, false),
        ];
        windows.extend(openh264_encoder_offers());
        windows.push(CodecOffer::tiles());
        let choices = negotiate(&windows, &viewer, CodecPreference::Auto);
        let expected_first = if OPENH264_DECODES_HIGH {
            H264Profile::High
        } else {
            H264Profile::ConstrainedBaseline
        };
        assert_eq!(
            choices[0],
            CodecChoice {
                codec: CodecKind::H264,
                profile: Some(expected_first),
                encoder_hardware: true,
                decoder_hardware: false,
                max_size: FrameSize::MAX,
                max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
                max_macroblocks_per_second: None,
            }
        );
        assert!(choices
            .iter()
            .any(|choice| choice.profile == Some(H264Profile::ConstrainedBaseline)));
        assert_eq!(
            choices.last().map(|choice| choice.codec),
            Some(CodecKind::Tiles)
        );
        // An OpenH264 host with an OpenH264 viewer: Constrained Baseline
        // within the encoder's 3840 × 2160.
        let mut linux = openh264_encoder_offers();
        linux.push(CodecOffer::tiles());
        let choices = negotiate(&linux, &viewer, CodecPreference::Auto);
        assert_eq!(
            choices[0],
            CodecChoice {
                codec: CodecKind::H264,
                profile: Some(H264Profile::ConstrainedBaseline),
                encoder_hardware: false,
                decoder_hardware: false,
                max_size: OPENH264_MAX_ENCODE,
                max_pixels: OPENH264_MAX_ENCODE.pixels(),
                max_macroblocks_per_second: None,
            }
        );
        // A viewer with Constrained Baseline only never gets High.
        let cb_only = vec![cb];
        let choices = negotiate(&windows, &cb_only, CodecPreference::H264);
        assert!(!choices.is_empty());
        assert!(choices
            .iter()
            .all(|choice| choice.profile == Some(H264Profile::ConstrainedBaseline)));
    }
}
