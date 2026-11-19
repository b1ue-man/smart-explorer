//! The bit rate of the Media Foundation encoders (moved out of
//! `mf_media.rs` on 2026-11-17, plan 2026-11-17): the buffer of one frame
//! in bytes, the ceiling of an opened encoder, a changed rate at run time
//! and the read-back of the mean bit rate. An encoder that refuses its rate
//! or reads back another one is named so (`set_mean_bitrate`), and the
//! watch of its rate gives it up (`rate_control::honour`).
use std::io;

use windows::core::{GUID, VARIANT};
use windows::Win32::Media::MediaFoundation::{
    eAVEncCommonRateControlMode_CBR, eAVEncCommonRateControlMode_PeakConstrainedVBR,
    CODECAPI_AVEncCommonBufferSize, CODECAPI_AVEncCommonMaxBitRate,
    CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode, ICodecAPI, IMFTransform,
    MF_MT_MPEG2_LEVEL,
};

use super::super::mf_common::EncoderSetup;
use super::{range_max, set_value};
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::rate_control::MIN_BITRATE_BPS;

/// The CBR buffer of one frame at `bitrate` and `fps` (R01
/// `nvenc_base.cpp:260-283`, VBV = bit rate / frame rate) in bytes, the unit
/// of `CODECAPI_AVEncCommonBufferSize` for H.264 and HEVC (Microsoft's
/// property and encoder pages; AV1 as HEVC, assumption): plan 2026-11-17,
/// `rate_control::model::mf_buffer_bytes`.
pub(in crate::screen::os) fn one_frame_buffer(bitrate: u32, fps: u32) -> u32 {
    crate::screen::core::rate_control::model::mf_buffer_bytes(bitrate, fps)
}

/// MaxBR of H.264 Annex A Table A-1 in bit/s: 1000 bit/s units for
/// Baseline/Main, × 1.25 for High (`cpbBrVclFactor` 1250). `level` as in
/// `MF_MT_MPEG2_LEVEL` (`eAVEncH264VLevel`: 10 = 1, 9 = 1b, 51 = 5.1).
pub(in crate::screen::os) fn level_max_bitrate(level: u32, high: bool) -> Option<u64> {
    let kbits: u64 = match level {
        10 => 64,
        9 => 128,
        11 => 192,
        12 => 384,
        13 => 768,
        20 => 2_000,
        21 | 22 => 4_000,
        30 => 10_000,
        31 => 14_000,
        32 | 40 => 20_000,
        41 | 42 => 50_000,
        50 => 135_000,
        51 | 52 | 60 => 240_000,
        61 => 480_000,
        62 => 800_000,
        _ => return None,
    };
    Some(if high { kbits * 1250 } else { kbits * 1000 })
}

/// The bit rate ceiling of an opened encoder: the smaller of what its level
/// and the encoder report; without either only the `u32` of the API.
pub(in crate::screen::os) fn bitrate_limit(level: Option<u64>, encoder: Option<u64>) -> u32 {
    [level, encoder]
        .into_iter()
        .flatten()
        .filter(|limit| *limit >= MIN_BITRATE_BPS)
        .min()
        .unwrap_or(u64::from(u32::MAX))
        .min(u64::from(u32::MAX)) as u32
}

/// `target` within the floor of the rate control and `limit`.
pub(in crate::screen::os) fn bounded_bitrate(target: u64, limit: u32) -> u32 {
    target.clamp(MIN_BITRATE_BPS, u64::from(limit).max(MIN_BITRATE_BPS)) as u32
}

/// The value the encoder reads back for `key`; `None` when it does not tell.
fn read_value(api: &ICodecAPI, key: &GUID) -> Option<u32> {
    // SAFETY: a read of one property into an owned VARIANT.
    let value = unsafe { api.GetValue(key) }.ok()?;
    u32::try_from(&value).ok()
}

/// Sets the mean bit rate and reads it back (plan 2026-11-17): `true` when
/// the encoder refuses it or reads back a value more than 2 % off.
pub(in crate::screen::os) fn set_mean_bitrate(api: &ICodecAPI, bitrate: u32) -> bool {
    let key = &CODECAPI_AVEncCommonMeanBitRate;
    if !set_value(api, key, VARIANT::from(bitrate)) {
        return true;
    }
    read_value(api, key)
        .is_some_and(|read| u64::from(read.abs_diff(bitrate)) * 50 > u64::from(bitrate))
}

/// The bit rate ceiling of the opened `transform` (after its types are set);
/// the H.264 level table only for H.264 (HEVC and AV1: the encoder's range,
/// the pairing's ceiling is the rate control's, merge W1).
pub(in crate::screen::os) fn opened_limit(
    transform: &IMFTransform,
    api: Option<&ICodecAPI>,
    setup: &EncoderSetup,
) -> u32 {
    let h264 = setup.codec == crate::screen::core::negotiate::CodecKind::H264;
    // SAFETY: plain reads of the transform's current output type.
    let level = unsafe { transform.GetOutputCurrentType(0) }
        .and_then(|media| unsafe { media.GetUINT32(&MF_MT_MPEG2_LEVEL) })
        .ok()
        .filter(|_| h264);
    let high = setup.profile == H264Profile::High;
    let by_level = level.and_then(|level| level_max_bitrate(level, high));
    let by_encoder = api
        .and_then(|api| range_max(api, &CODECAPI_AVEncCommonMeanBitRate))
        .map(u64::from);
    bitrate_limit(by_level, by_encoder)
}

/// Sets a changed bit rate (at least 2 % off) within `limit` and its one
/// frame buffer; a refusal keeps the old one. With `peak` (sharp mode,
/// `peak_mode`) the peak rate moves with it and stays at or above the mean.
/// `None` when nothing was set, else whether the encoder refused the rate
/// or reads back another one (`set_mean_bitrate`, plan 2026-11-17).
pub(in crate::screen::os) fn retune_bounded(
    api: Option<&ICodecAPI>,
    current: &mut u32,
    target_bps: u64,
    limit: u32,
    fps: u32,
    peak: bool,
) -> Option<bool> {
    let target = bounded_bitrate(target_bps, limit);
    // Review 2026-11-19 L1: the 2 % band holds back rises only; every
    // lowering is set, so the encoder never stays above a lowered rate.
    if target >= *current && (i64::from(target) - i64::from(*current)) * 50 < i64::from(*current) {
        return None;
    }
    // Codex 14 (review 2026-11-19): without `ICodecAPI` a lowering cannot
    // be set; it is a refused one, not nothing to do.
    let Some(api) = api else {
        return (target < *current).then_some(true);
    };
    let raising = target > *current;
    if peak && raising && !set_max_bitrate(api, target) {
        return Some(true);
    }
    if set_mean_bitrate(api, target) {
        return Some(true);
    }
    if peak && !raising && !set_max_bitrate(api, target) {
        return Some(true);
    }
    *current = target;
    let buffer = VARIANT::from(one_frame_buffer(target, fps));
    set_value(api, &CODECAPI_AVEncCommonBufferSize, buffer);
    Some(false)
}

/// Review 2026-11-19 B6: after a refused rate change, an encoder that still
/// runs at `running` above the lowered target of `target_bps` would keep
/// exceeding the rate the session (and the user's ceiling) allows. That
/// encoder fails its frame instead: the stage opens a fresh one at the
/// lowered rate (`Running::failure_refuses` of the host process) or the
/// next encoder of the codec, or names the failure.
pub(in crate::screen::os) fn refused_lowering(
    running: u32,
    target_bps: u64,
    limit: u32,
) -> io::Result<()> {
    let target = bounded_bitrate(target_bps, limit);
    if target < running {
        return Err(io::Error::other(format!(
            "the encoder refused to lower its bit rate from {running} to {target} bit/s"
        )));
    }
    Ok(())
}

/// P9, sharp mode, on a Media Foundation encoder (review 2026-11-19 B2):
/// `eAVEncCommonRateControlMode_Quality` keeps no rate at all (its mean bit
/// rate is not used and Microsoft documents no peak for it), so the session
/// rate would no longer bound the stream. Sharp mode is therefore
/// peak-constrained VBR (`eAVEncCommonRateControlMode_PeakConstrainedVBR`)
/// with the session rate as mean and peak (`CODECAPI_AVEncCommonMaxBitRate`),
/// as the GPU vendors' sharp mode keeps the session rate as its peak. An MFT
/// that refuses either keeps CBR. Whether it was taken.
fn peak_mode(api: &ICodecAPI, bitrate: u32) -> bool {
    let key = &CODECAPI_AVEncCommonRateControlMode;
    let peak = eAVEncCommonRateControlMode_PeakConstrainedVBR.0 as u32;
    if set_value(api, key, VARIANT::from(peak)) && set_max_bitrate(api, bitrate) {
        return true;
    }
    let cbr = eAVEncCommonRateControlMode_CBR.0 as u32;
    set_value(api, key, VARIANT::from(cbr));
    false
}

/// Sharp mode of an opened MFT when the session asks for it (`sharp`):
/// `peak_mode`, named in the host log; whether the MFT runs it. Review
/// 2026-11-19 (Gemini 7): an MFT that refuses it fails its opening with the
/// reason (as VA-API, `SHARP_UNSUPPORTED`) instead of encoding CBR in
/// silence; the stage then tries the next GPU encoder or names the failure.
pub(in crate::screen::os) fn sharp_mode(
    api: Option<&ICodecAPI>,
    sharp: bool,
    bitrate: u32,
    name: &str,
) -> io::Result<bool> {
    if !sharp {
        return Ok(false);
    }
    if !api.is_some_and(|api| peak_mode(api, bitrate)) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "sharp_quality_unsupported: the Media Foundation encoder {name} refuses \
                 peak-constrained VBR"
            ),
        ));
    }
    eprintln!(
        "screen encoder {name}: sharp mode as peak-constrained VBR (Media Foundation keeps no rate in its quality mode)"
    );
    Ok(true)
}

/// Sets the peak bit rate; `true` when the encoder takes it and reads back
/// no higher one (or does not tell).
fn set_max_bitrate(api: &ICodecAPI, bitrate: u32) -> bool {
    let key = &CODECAPI_AVEncCommonMaxBitRate;
    set_value(api, key, VARIANT::from(bitrate))
        && read_value(api, key).is_none_or(|read| u64::from(read) * 50 <= u64::from(bitrate) * 51)
}
