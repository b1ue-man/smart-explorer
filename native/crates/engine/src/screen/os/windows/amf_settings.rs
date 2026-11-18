//! The settings of the AMF encoders (BK12, B-21 and B-22; HEVC and AV1 by
//! merge W1) as property lists, so they are checked without a GPU. Names
//! and values: `amf_abi.rs` (`VideoEncoderVCE.h`, `ColorSpace.h` of AMF SDK
//! 1.4.35) and `amf_codecs.rs` (`VideoEncoderHEVC.h`, `VideoEncoderAV1.h`).
//! Order and split follow FFmpeg `amfenc_h264.c` (n7.1): `Usage` first (it
//! "fully configures parameter set", `VideoEncoderVCE.h:187`), then the
//! static properties before `Init`, the remaining ones after it
//! (`:486-518`). The colour signal is BT.709 limited range for every codec,
//! as the NV12 of the capture ring (`d3d_video_convert.rs`).
use super::amf_abi::{prop, AmfVariant};
use super::amf_codecs::{self as codecs, Names};
use super::vendor_encode::{avc_level_for, RateParams};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

/// `QueryTimeout` in ms: `QueryOutput` waits inside the driver.
pub(super) const QUERY_TIMEOUT_MS: i64 = 50;
/// Long-term references for B-36 (`MaxOfLTRFrames`).
pub(super) const LTR_FRAMES: i64 = 2;

pub(super) type Setting = (&'static str, AmfVariant);

/// Properties before `Init`. P9: with a quality target (sharp mode) the
/// rate control is quality VBR at that level, the session rate its peak;
/// pre-analysis and pre-encode stay off in either mode.
pub(super) fn static_settings(names: &Names, size: FrameSize, rate: &RateParams) -> Vec<Setting> {
    let int = AmfVariant::int64;
    let color = int(prop::COLOR_PROFILE_709);
    let method = if rate.quality.is_some() {
        names.quality_vbr
    } else {
        names.cbr
    };
    let pre_encode = if names.codec == CodecKind::H264 {
        int(0)
    } else {
        AmfVariant::boolean(false)
    };
    let mut settings = vec![
        (names.usage, int(names.usage_ultra_low_latency)),
        (names.profile, int(names.profile_value)),
        (names.quality, int(names.quality_speed)),
        (names.framesize, AmfVariant::size(size.width, size.height)),
        (names.max_ltr, int(LTR_FRAMES)),
        (names.ltr_mode, int(names.ltr_keep_unused)),
        (names.rate_control, int(method)),
        (names.pre_analysis, AmfVariant::boolean(false)),
        (names.pre_encode, pre_encode),
        (names.in_color_profile, color),
        (names.out_color_profile, color),
        (names.out_transfer, int(prop::TRANSFER_BT709)),
        (names.out_primaries, int(prop::PRIMARIES_BT709)),
    ];
    match names.codec {
        CodecKind::H264 => settings.extend([
            (
                prop::PROFILE_LEVEL,
                int(i64::from(avc_level_for(size, rate.fps))),
            ),
            (prop::LOWLATENCY_MODE, AmfVariant::boolean(true)),
            (prop::B_PIC_PATTERN, int(0)),
            (prop::FULL_RANGE_COLOR, AmfVariant::boolean(false)),
        ]),
        CodecKind::Hevc => settings.extend([
            (codecs::HEVC_TIER, int(0)),
            (codecs::HEVC_LOW_LATENCY, AmfVariant::boolean(true)),
            (codecs::HEVC_NOMINAL_RANGE, int(0)),
            (codecs::HEVC_GOP_SIZE, int(i64::from(i16::MAX))),
            (codecs::HEVC_GOPS_PER_IDR, int(1)),
            (
                codecs::HEVC_HEADER_INSERTION,
                int(codecs::HEVC_HEADERS_AT_IDR),
            ),
        ]),
        CodecKind::Av1 => settings.extend([
            (codecs::AV1_LATENCY, int(codecs::AV1_LOWEST_LATENCY)),
            (codecs::AV1_ALIGNMENT, int(codecs::AV1_NO_ALIGNMENT)),
            (names.query_timeout, int(QUERY_TIMEOUT_MS)),
        ]),
        CodecKind::Tiles => {}
    }
    if let Some(quality) = rate.quality {
        settings.push((names.qvbr_level, int(i64::from(quality))));
    }
    settings.extend(rate_settings(names, rate));
    settings
}

/// The rate of B-21 (CBR, target = peak, VBV of one frame, full at the
/// start); dynamic properties, so a new rate needs no restart.
pub(super) fn rate_settings(names: &Names, rate: &RateParams) -> Vec<Setting> {
    let bits = i64::from(rate.bits_per_second);
    vec![
        (names.target, AmfVariant::int64(bits)),
        (names.peak, AmfVariant::int64(bits)),
        (names.vbv, AmfVariant::int64(i64::from(rate.buffer_bits))),
        (names.vbv_fullness, AmfVariant::int64(prop::VBV_FULL)),
        (names.framerate, AmfVariant::rate(rate.fps, 1)),
    ]
}

/// Properties after `Init`: no periodic IDR, HRD enforced (the one-frame
/// buffer holds), no filler data, no skipped frames, waiting output.
pub(super) fn dynamic_settings(names: &Names) -> Vec<Setting> {
    let mut settings = Vec::new();
    match names.codec {
        CodecKind::H264 => settings.push((prop::IDR_PERIOD, AmfVariant::int64(0))),
        CodecKind::Av1 => settings.extend([
            (codecs::AV1_GOP_SIZE, AmfVariant::int64(0)),
            (
                codecs::AV1_HEADER_INSERTION,
                AmfVariant::int64(codecs::AV1_HEADERS_AT_KEY),
            ),
        ]),
        CodecKind::Hevc | CodecKind::Tiles => {}
    }
    settings.extend([
        (names.enforce_hrd, AmfVariant::boolean(true)),
        (names.filler, AmfVariant::boolean(false)),
        (names.skip, AmfVariant::boolean(false)),
    ]);
    if names.codec != CodecKind::Av1 {
        // AV1's timeout is a static property (`VideoEncoderAV1.h:194`).
        settings.push((names.query_timeout, AmfVariant::int64(QUERY_TIMEOUT_MS)));
    }
    settings
}

/// The per-picture properties of a requested key frame: its type and the
/// parameter sets before it.
pub(super) fn key_settings(names: &Names) -> Vec<Setting> {
    let mut settings = vec![(names.force_type, AmfVariant::int64(names.force_key))];
    settings.extend(
        names
            .insert_headers
            .iter()
            .map(|name| (*name, AmfVariant::boolean(true))),
    );
    settings
}
