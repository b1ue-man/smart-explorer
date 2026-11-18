//! The codec arguments of `screen.viewOpen` (moved out of `screen_view.rs`;
//! plan 2026-11-14 „Bildkette“ BK6): the app's decoders and the viewer's
//! „Codec“ and „Dekodierer“ (4.10).
//!
//! Fields the app sends (BK7b fills them):
//! - `decoders[]`: `codec` `h264` (with `profile` `cb`/`high`), `hevc` or
//!   `av1` (with `profile` `main`, or `main_444`/`high` for 4:4:4), or
//!   `tiles`; `hardware`, `pixelAccess`, `maxWidth`, `maxHeight`,
//!   `maxMacroblocks`, `maxMacroblocksPerSecond` as for H.264.
//! - `codec`: `auto`, `h264`, `hevc`, `av1`, `sharp` or `lossless` (absent:
//!   automatic; `lossless` is the switch „Verlustfrei“ and travels as
//!   `SET_MODE`; `sharp` is „Scharf (GPU)“, H.264/HEVC on the host's GPU
//!   encoder, `SET_MODE` `sharpness`, plan 2026-11-17 P9).
//! - `decoder`: `auto`, `hardware` or `software` (absent: automatic). The
//!   decoders run in the app (MediaCodec), so the app also chooses its
//!   decoder stage itself; the core only narrows the offers.
//! - `virtualDisplay`: `auto`, `off` or `request` and the fixed size
//!   `virtualWidth` × `virtualHeight` (plan 2026-11-17 P5, „Monitor“ and
//!   „Auflösung“ of the desktop viewer); absent: no wish, the host keeps
//!   its display as before.
//! - `maxRate`: „Höchste Datenrate“ in bit/s, at least the host's floor
//!   `MIN_BITRATE_BPS` (0,15 Mbit/s): a lower rate is refused with its
//!   reason instead of being raised by the host without a word (user rule
//!   of 2026-11-18: a choice is used exactly as made).
use serde_json::Value;

use super::super::args::{invalid, opt_bool, opt_i64, opt_str};
use super::size_args;
use crate::mobile::ApiError;
use crate::screen::core::negotiate::{
    CodecKind, CodecOffer, CodecSetting, DecoderPreference, H264Profile,
};
use crate::screen::core::rate_control::MIN_BITRATE_BPS;
use crate::screen::core::virtual_display::{VirtualDisplayChoice, VirtualDisplayWish};
use crate::screen::core::wire_hello::SessionMode;

/// The profile of an HEVC or AV1 entry: 4:2:0 (`None`) or 4:4:4.
fn video_profile(codec: CodecKind, entry: &Value) -> Result<Option<H264Profile>, ApiError> {
    match (codec, opt_str(entry, "profile")) {
        (_, None | Some("main")) => Ok(None),
        (CodecKind::Hevc, Some("main_444")) | (CodecKind::Av1, Some("high")) => {
            Ok(Some(H264Profile::High))
        }
        _ => Err(invalid(
            "Decoder: „profile“ ist main, main_444 (HEVC) oder high (AV1).",
        )),
    }
}

pub(super) fn decoders_arg(args: &Value, mode: SessionMode) -> Result<Vec<CodecOffer>, ApiError> {
    let mut offers = Vec::new();
    for entry in args
        .get("decoders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let hardware = opt_bool(entry, "hardware", false);
        let offer = match opt_str(entry, "codec") {
            Some("tiles") => {
                offers.push(CodecOffer::tiles());
                continue;
            }
            Some("h264") => {
                let profile = match opt_str(entry, "profile") {
                    Some("cb") => H264Profile::ConstrainedBaseline,
                    Some("high") => H264Profile::High,
                    _ => return Err(invalid("Decoder: „profile“ ist cb oder high.")),
                };
                CodecOffer::h264(profile, hardware)
            }
            Some(key @ ("hevc" | "av1")) => {
                let codec = if key == "hevc" {
                    CodecKind::Hevc
                } else {
                    CodecKind::Av1
                };
                let profile = video_profile(codec, entry)?;
                CodecOffer::video(codec, profile.is_some(), hardware)
            }
            _ => return Err(invalid("Decoder: „codec“ ist h264, hevc, av1 oder tiles.")),
        };
        let offer = offer.with_pixel_access(opt_bool(entry, "pixelAccess", false));
        let mut offer =
            size_args(entry, "maxWidth", "maxHeight").map_or(offer, |max| offer.with_max(max));
        offer.max_macroblocks = entry
            .get("maxMacroblocks")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        offer.max_macroblocks_per_second = entry
            .get("maxMacroblocksPerSecond")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        offers.push(offer);
    }
    if !offers.iter().any(|offer| offer.codec == CodecKind::Tiles) {
        offers.push(CodecOffer::tiles());
    }
    if mode == SessionMode::Preview {
        offers.retain(|offer| offer.codec == CodecKind::Tiles);
    }
    Ok(offers)
}

/// `codec` and `decoder` of `screen.viewOpen` (4.10); absent is automatic.
pub(super) fn codec_args(
    args: &Value,
) -> Result<(Option<CodecSetting>, Option<DecoderPreference>), ApiError> {
    let codec = opt_str(args, "codec")
        .map(|text| {
            CodecSetting::parse(text)
                .ok_or_else(|| invalid("„codec“ ist auto, h264, hevc, av1, sharp oder lossless."))
        })
        .transpose()?;
    let decoder = opt_str(args, "decoder")
        .map(|text| {
            DecoderPreference::parse(text)
                .ok_or_else(|| invalid("„decoder“ ist auto, hardware oder software."))
        })
        .transpose()?;
    Ok((codec, decoder))
}

/// `virtualDisplay` and `virtualWidth` × `virtualHeight` of
/// `screen.viewOpen` (plan 2026-11-17 P5); `None` without `virtualDisplay`.
pub(super) fn virtual_display_arg(args: &Value) -> Result<Option<VirtualDisplayChoice>, ApiError> {
    let Some(text) = opt_str(args, "virtualDisplay") else {
        return Ok(None);
    };
    let wish = VirtualDisplayWish::parse(text)
        .ok_or_else(|| invalid("„virtualDisplay“ ist auto, off oder request."))?;
    let choice = VirtualDisplayChoice {
        wish,
        size: size_args(args, "virtualWidth", "virtualHeight"),
    };
    choice
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    Ok(Some(choice))
}

/// `maxRate` of `screen.viewOpen` (`zero_automatic` false: absent is
/// automatic) and of `screen.viewSetMode` (`zero_automatic` true: 0 is
/// automatic, `Some(None)`); `None` without `maxRate`. A rate below
/// `MIN_BITRATE_BPS` is an error, not a rate the host raises silently.
pub(super) fn max_rate_arg(
    args: &Value,
    zero_automatic: bool,
) -> Result<Option<Option<u64>>, ApiError> {
    let Some(rate) = opt_i64(args, "maxRate") else {
        return Ok(None);
    };
    if zero_automatic && rate == 0 {
        return Ok(Some(None));
    }
    match u64::try_from(rate) {
        Ok(rate) if rate >= MIN_BITRATE_BPS => Ok(Some(Some(rate))),
        _ => Err(invalid(format!(
            "„maxRate“ ist {}mindestens {MIN_BITRATE_BPS} bit/s (0,15 Mbit/s): \
             darunter arbeitet der Host nicht und würde die Rate von selbst anheben.",
            if zero_automatic {
                "0 (automatisch) oder "
            } else {
                ""
            }
        ))),
    }
}
