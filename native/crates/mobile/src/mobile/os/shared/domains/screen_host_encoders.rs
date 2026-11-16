//! The encoders the app reports in `screen.hostPrereqs` (plan 2026-11-14 „Bildkette“ BK7b, B-26).
//!
//! `encoders[]` entries: `codec` `h264` (the default of an app before BK7b, with `profile` `cb` or
//! `high`), `hevc` or `av1` (with `profile` `main` for 4:2:0, `main_444` (HEVC) or `high` (AV1) for
//! 4:4:4); `hardware`, `maxWidth`, `maxHeight`, `maxMacroblocks`, `maxMacroblocksPerSecond`. The
//! app reports an HEVC or AV1 encoder only when MediaCodec names a hardware encoder for it. An entry
//! the core does not know is skipped, never an error: a newer app stays usable.
use serde_json::Value;

use super::super::args::{opt_bool, opt_str};
use crate::screen::core::negotiate::{CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

/// One side limit of an entry (1 to 16384 pixels).
fn side(entry: &Value, key: &str) -> Option<u32> {
    entry
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0 && *n <= 16_384)
}

/// The offer an entry names, `None` for a codec or profile this core does not know.
fn offer_of(entry: &Value) -> Option<CodecOffer> {
    let hardware = opt_bool(entry, "hardware", false);
    let offer = match (
        opt_str(entry, "codec").unwrap_or("h264"),
        opt_str(entry, "profile"),
    ) {
        ("h264", Some("cb")) => CodecOffer::h264(H264Profile::ConstrainedBaseline, hardware),
        ("h264", Some("high")) => CodecOffer::h264(H264Profile::High, hardware),
        ("hevc", None | Some("main")) => CodecOffer::video(CodecKind::Hevc, false, hardware),
        ("hevc", Some("main_444")) => CodecOffer::video(CodecKind::Hevc, true, hardware),
        ("av1", None | Some("main")) => CodecOffer::video(CodecKind::Av1, false, hardware),
        ("av1", Some("high")) => CodecOffer::video(CodecKind::Av1, true, hardware),
        _ => return None,
    };
    let mut offer = match (side(entry, "maxWidth"), side(entry, "maxHeight")) {
        (Some(width), Some(height)) => offer.with_max(FrameSize::new(width, height)),
        _ => offer,
    };
    offer.max_macroblocks = entry
        .get("maxMacroblocks")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0);
    offer.max_macroblocks_per_second = entry
        .get("maxMacroblocksPerSecond")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0);
    Some(offer)
}

/// The offers of the `encoders` list, in the app's order.
pub(super) fn encoders_arg(list: &[Value]) -> Vec<CodecOffer> {
    list.iter().filter_map(offer_of).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bildkette_task_android_host_encoders_name_their_codec() {
        let list = json!([
            // An app before BK7b names no codec: H.264.
            {"profile": "high", "hardware": true, "maxWidth": 1920, "maxHeight": 1088,
             "maxMacroblocks": 8160, "maxMacroblocksPerSecond": 244800},
            {"codec": "h264", "profile": "cb"},
            {"codec": "hevc", "profile": "main", "hardware": true, "maxWidth": 3840, "maxHeight": 2160},
            {"codec": "av1", "hardware": true},
            // Unknown codecs and profiles are skipped, not an error.
            {"codec": "vp9", "profile": "main", "hardware": true},
            {"codec": "hevc", "profile": "bogus"},
            {"codec": "h264"},
        ]);
        let offers = encoders_arg(list.as_array().unwrap());
        let mut first =
            CodecOffer::h264(H264Profile::High, true).with_max(FrameSize::new(1920, 1088));
        first.max_macroblocks = Some(8160);
        first.max_macroblocks_per_second = Some(244_800);
        assert_eq!(
            offers,
            vec![
                first,
                CodecOffer::h264(H264Profile::ConstrainedBaseline, false),
                CodecOffer::video(CodecKind::Hevc, false, true)
                    .with_max(FrameSize::new(3840, 2160)),
                CodecOffer::video(CodecKind::Av1, false, true),
            ]
        );
        // The host's hello keeps HEVC and AV1 out of the legacy list (B-31).
        assert_eq!(
            offers
                .iter()
                .filter(|offer| offer.codec.is_legacy())
                .count(),
            2
        );
    }

    #[test]
    fn bildkette_task_android_host_encoders_take_444_only_by_its_profile_name() {
        let list = json!([
            {"codec": "hevc", "profile": "main_444", "hardware": true},
            {"codec": "av1", "profile": "high", "hardware": true},
            {"codec": "hevc", "profile": "high", "hardware": true},
            {"codec": "av1", "profile": "main_444", "hardware": true},
        ]);
        assert_eq!(
            encoders_arg(list.as_array().unwrap()),
            vec![
                CodecOffer::video(CodecKind::Hevc, true, true),
                CodecOffer::video(CodecKind::Av1, true, true),
            ]
        );
    }
}
