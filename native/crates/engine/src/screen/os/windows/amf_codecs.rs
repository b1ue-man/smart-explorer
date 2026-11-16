//! The property names and values of the three AMF encoder components
//! (merge W1: H.264 of BK12, HEVC and AV1 for plan 2026-11-14 „Bildkette“
//! B-23/B-30). Source: AMF SDK tag `v1.4.35`,
//! `components/VideoEncoderVCE.h` (via `amf_abi::prop`),
//! `components/VideoEncoderHEVC.h` and `components/VideoEncoderAV1.h`
//! (GPUOpen-LibrariesAndSDKs/AMF, read on 2026-11-14 from jsDelivr); the
//! numbers in brackets are header lines. Enum values are counted from the
//! header where it gives none (`HEVC_USAGE_ULTRA_LOW_LATENCY` = 1 after
//! `TRANSCODING = 0`, `HEVC_RATE_CONTROL_METHOD_CBR` = 3 after `UNKNOWN =
//! -1`, `HEVC_PICTURE_TYPE_IDR` = 2, `HEVC_OUTPUT_DATA_TYPE_IDR` = 0).
use super::amf_abi::{full_version, prop, ENCODER_AVC};
use crate::screen::core::negotiate::CodecKind;

/// One AMF encoder component: the names its properties have and the values
/// B-21 sets.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Names {
    pub codec: CodecKind,
    pub component: &'static str,
    /// Oldest runtime for this component (AV1: 1.4.30, Sunshine
    /// `display_vram.cpp:2079`: ultra low latency for AV1 from 1.4.30).
    pub min_version: u64,
    pub usage: &'static str,
    pub usage_ultra_low_latency: i64,
    pub profile: &'static str,
    pub profile_value: i64,
    pub quality: &'static str,
    pub quality_speed: i64,
    pub framesize: &'static str,
    pub rate_control: &'static str,
    pub cbr: i64,
    pub target: &'static str,
    pub peak: &'static str,
    pub vbv: &'static str,
    pub vbv_fullness: &'static str,
    pub framerate: &'static str,
    pub enforce_hrd: &'static str,
    pub filler: &'static str,
    pub skip: &'static str,
    pub query_timeout: &'static str,
    pub max_ltr: &'static str,
    pub ltr_mode: &'static str,
    pub ltr_keep_unused: i64,
    pub mark_ltr: &'static str,
    pub force_ltr: &'static str,
    pub force_type: &'static str,
    pub force_key: i64,
    /// Per-picture properties that put the parameter sets (SPS/PPS, VPS,
    /// sequence header) before a requested key frame.
    pub insert_headers: &'static [&'static str],
    pub output_type: &'static str,
    pub output_key: i64,
    pub in_color_profile: &'static str,
    pub out_color_profile: &'static str,
    pub out_transfer: &'static str,
    pub out_primaries: &'static str,
}

/// `AMFVideoEncoderVCE_AVC` (BK12).
pub(super) const AVC: Names = Names {
    codec: CodecKind::H264,
    component: ENCODER_AVC,
    min_version: full_version(1, 4, 0, 0),
    usage: prop::USAGE,
    usage_ultra_low_latency: prop::USAGE_ULTRA_LOW_LATENCY,
    profile: prop::PROFILE,
    profile_value: prop::PROFILE_HIGH,
    quality: prop::QUALITY_PRESET,
    quality_speed: prop::QUALITY_SPEED,
    framesize: prop::FRAMESIZE,
    rate_control: prop::RATE_CONTROL_METHOD,
    cbr: prop::RATE_CONTROL_CBR,
    target: prop::TARGET_BITRATE,
    peak: prop::PEAK_BITRATE,
    vbv: prop::VBV_BUFFER_SIZE,
    vbv_fullness: prop::INITIAL_VBV_FULLNESS,
    framerate: prop::FRAMERATE,
    enforce_hrd: prop::ENFORCE_HRD,
    filler: prop::FILLER_DATA_ENABLE,
    skip: prop::SKIP_FRAME_ENABLE,
    query_timeout: prop::QUERY_TIMEOUT,
    max_ltr: prop::MAX_LTR_FRAMES,
    ltr_mode: prop::LTR_MODE,
    ltr_keep_unused: prop::LTR_KEEP_UNUSED,
    mark_ltr: prop::MARK_LTR_INDEX,
    force_ltr: prop::FORCE_LTR_BITFIELD,
    force_type: prop::FORCE_PICTURE_TYPE,
    force_key: prop::PICTURE_TYPE_IDR,
    insert_headers: &[prop::INSERT_SPS, prop::INSERT_PPS],
    output_type: prop::OUTPUT_DATA_TYPE,
    output_key: prop::OUTPUT_IDR,
    in_color_profile: prop::INPUT_COLOR_PROFILE,
    out_color_profile: prop::OUTPUT_COLOR_PROFILE,
    out_transfer: prop::OUTPUT_TRANSFER,
    out_primaries: prop::OUTPUT_PRIMARIES,
};

/// `AMFVideoEncoderHW_HEVC` [35], properties [153-266].
pub(super) const HEVC: Names = Names {
    codec: CodecKind::Hevc,
    component: "AMFVideoEncoderHW_HEVC",
    min_version: full_version(1, 4, 0, 0),
    usage: "HevcUsage",
    usage_ultra_low_latency: 1,
    profile: "HevcProfile",
    profile_value: 1,
    quality: "HevcQualityPreset",
    quality_speed: 10,
    framesize: "HevcFrameSize",
    rate_control: "HevcRateControlMethod",
    cbr: 3,
    target: "HevcTargetBitrate",
    peak: "HevcPeakBitrate",
    vbv: "HevcVBVBufferSize",
    vbv_fullness: "HevcInitialVBVBufferFullness",
    framerate: "HevcFrameRate",
    enforce_hrd: "HevcEnforceHRD",
    filler: "HevcFillerDataEnable",
    skip: "HevcRateControlSkipFrameEnable",
    query_timeout: "HevcQueryTimeout",
    max_ltr: "HevcMaxOfLTRFrames",
    ltr_mode: "HevcLTRMode",
    ltr_keep_unused: 1,
    mark_ltr: "HevcMarkCurrentWithLTRIndex",
    force_ltr: "HevcForceLTRReferenceBitfield",
    force_type: "HevcForcePictureType",
    force_key: 2,
    insert_headers: &["HevcInsertHeader"],
    output_type: "HevcOutputDataType",
    output_key: 0,
    in_color_profile: "HevcInColorProfile",
    out_color_profile: "HevcOutColorProfile",
    out_transfer: "HevcOutColorTransferChar",
    out_primaries: "HevcOutColorPrimaries",
};

/// `AMFVideoEncoderHW_AV1` [35], properties [193-307].
pub(super) const AV1: Names = Names {
    codec: CodecKind::Av1,
    component: "AMFVideoEncoderHW_AV1",
    min_version: full_version(1, 4, 30, 0),
    usage: "Av1Usage",
    usage_ultra_low_latency: 2,
    profile: "Av1Profile",
    profile_value: 1,
    quality: "Av1QualityPreset",
    quality_speed: 100,
    framesize: "Av1FrameSize",
    rate_control: "Av1RateControlMethod",
    cbr: 3,
    target: "Av1TargetBitrate",
    peak: "Av1PeakBitrate",
    vbv: "Av1VBVBufferSize",
    vbv_fullness: "Av1InitialVBVBufferFullness",
    framerate: "Av1FrameRate",
    enforce_hrd: "Av1EnforceHRD",
    filler: "Av1FillerData",
    skip: "Av1RateControlSkipFrameEnable",
    query_timeout: "Av1QueryTimeout",
    max_ltr: "Av1MaxNumLTRFrames",
    ltr_mode: "Av1LTRMode",
    ltr_keep_unused: 1,
    mark_ltr: "Av1MarkCurrentWithLTRIndex",
    force_ltr: "Av1ForceLTRReferenceBitfield",
    force_type: "Av1ForceFrameType",
    force_key: 1,
    insert_headers: &["Av1ForceInsertSequenceHeader"],
    output_type: "Av1OutputFrameType",
    output_key: 0,
    in_color_profile: "Av1InputColorProfile",
    out_color_profile: "Av1OutputColorProfile",
    out_transfer: "Av1OutputColorTransferChar",
    out_primaries: "Av1OutputColorPrimaries",
};

/// HEVC: Main tier [157], the studio range [167], the GOP of Sunshine
/// (`gop_size` = `i16::MAX`, one GOP per IDR [171-172]), VPS/SPS/PPS before
/// every IDR (`HEADER_INSERTION_MODE_IDR_ALIGNED` = 2 [116], [175]).
pub(super) const HEVC_TIER: &str = "HevcTier";
pub(super) const HEVC_LOW_LATENCY: &str = "LowLatencyInternal";
pub(super) const HEVC_NOMINAL_RANGE: &str = "HevcNominalRange";
pub(super) const HEVC_GOP_SIZE: &str = "HevcGOPSize";
pub(super) const HEVC_GOPS_PER_IDR: &str = "HevcGOPSPerIDR";
pub(super) const HEVC_HEADER_INSERTION: &str = "HevcHeaderInsertionMode";
pub(super) const HEVC_HEADERS_AT_IDR: i64 = 2;
/// AV1: lowest encoding latency (`ENCODING_LATENCY_MODE_LOWEST_LATENCY` = 3
/// [42], B-29 latency before power), no alignment padding
/// (`ALIGNMENT_MODE_NO_RESTRICTIONS` = 3 [104]), key frames only on request
/// (`GOP_SIZE` 0 [275]), the sequence header with every key frame
/// (`HEADER_INSERTION_MODE_KEY_FRAME_ALIGNED` = 2 [137], [277]).
pub(super) const AV1_LATENCY: &str = "Av1EncodingLatencyMode";
pub(super) const AV1_LOWEST_LATENCY: i64 = 3;
pub(super) const AV1_ALIGNMENT: &str = "Av1AlignmentMode";
pub(super) const AV1_NO_ALIGNMENT: i64 = 3;
pub(super) const AV1_GOP_SIZE: &str = "Av1GOPSize";
pub(super) const AV1_HEADER_INSERTION: &str = "Av1HeaderInsertionMode";
pub(super) const AV1_HEADERS_AT_KEY: i64 = 2;

/// The component of `codec`; `None` for the tiles.
pub(super) fn names(codec: CodecKind) -> Option<&'static Names> {
    match codec {
        CodecKind::H264 => Some(&AVC),
        CodecKind::Hevc => Some(&HEVC),
        CodecKind::Av1 => Some(&AV1),
        CodecKind::Tiles => None,
    }
}
