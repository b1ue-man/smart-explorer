//! The NVENC encoder settings of both platforms (B-21 and R01; Linux BK4,
//! Windows BK12, one module since merge W2, plan 2026-11-14 „Bildkette“):
//! preset P1 with tuning "ultra low latency", CBR, VBV buffer of one frame,
//! no B pictures (`frameIntervalP` 1, `zeroReorderDelay`), no look-ahead,
//! infinite GOP with an IDR only on request, the parameter sets at every
//! IDR, BT.709 limited range in the VUI (Sunshine `nvenc_base.cpp:258-283`,
//! `:351-381`, `:641-643`). In addition (`apply_reference_settings`):
//! `lowDelayKeyFrameScale` 1 (Sunshine `:263`) and, for reference
//! invalidation (B-36), a DPB of 5 pictures (8 for AV1) with one reference
//! per picture (Sunshine `:291-298`, `:372`, `:408`, `:449`).
use super::abi::{Guid, NvEncConfig};
use super::api::{
    H264_REPEAT_SPSPPS, NVENC_INFINITE_GOPLENGTH, NV_ENC_CONFIG_VER, NV_ENC_H264_PROFILE_HIGH_GUID,
    NV_ENC_NUM_REF_FRAMES_1, NV_ENC_PARAMS_RC_CBR, NV_ENC_RC_PARAMS_VER, RC_ENABLE_LOOKAHEAD,
    RC_ZERO_REORDER_DELAY, VUI_BT709,
};
use super::codecs::{apply_union, apply_union_references, set_hevc_chroma};
use crate::screen::core::negotiate::CodecKind;

/// `NV_ENC_HEVC_PROFILE_FREXT_GUID` {51ec32b5-1b4c-453c-9cbd-b616bd621341}:
/// HEVC Main 4:4:4 (`nvEncodeAPI.h` of nv-codec-headers `n12.1.14.0`,
/// lines 201–204, checked 2026-11-18).
pub(in crate::screen::os) const NV_ENC_HEVC_PROFILE_FREXT_GUID: Guid = Guid {
    data1: 0x51ec_32b5,
    data2: 0x1b4c,
    data3: 0x453c,
    data4: [0x9c, 0xbd, 0xb6, 0x16, 0xbd, 0x62, 0x13, 0x41],
};

/// The name of the H.264 factory; HEVC and AV1 add their codec key
/// (`codecs::factory_name`).
pub(in crate::screen::os) const NVENC_NAME: &str = "nvenc";
/// DPB size with reference invalidation (Sunshine's default of 5,
/// `nvenc_base.cpp:372`); the header recommends "a large DPB size" for it
/// (`nvEncodeAPI.h:1734-1737`).
pub(in crate::screen::os) const RFI_DPB: u32 = 5;

/// The constant bit rate a platform's rate parameters name (Linux
/// `hw_encode_common::RateParams`, Windows `vendor_encode::RateParams`),
/// and the chroma of the stream they open.
pub(in crate::screen::os) trait NvRate {
    /// Bits per second, VBV buffer size and its initial fullness in bits.
    fn cbr(&self) -> (u32, u32, u32);
    /// Whether the stream has 4:4:4 chroma (HEVC Main 4:4:4, the wire
    /// profile `high` of B-32, mode „Scharf“ of plan 2026-11-17 P9); the
    /// encoder's input buffer must then be YUV444 (`chromaFormatIDC` 3 "for
    /// yuv444 input", `nvEncodeAPI.h:1795`). 4:2:0 unless a platform says so.
    fn chroma_444(&self) -> bool {
        false
    }
}

/// Applies B-21 to a preset configuration of `codec` for `rate` (H.264:
/// `apply_settings`; HEVC and AV1: the common part and `apply_union`).
pub(in crate::screen::os) fn apply_codec_settings(
    config: &mut NvEncConfig,
    rate: &impl NvRate,
    codec: CodecKind,
) {
    if codec == CodecKind::H264 {
        apply_settings(config, rate);
    } else {
        apply_common(config, rate);
        apply_union(config, codec, NVENC_INFINITE_GOPLENGTH);
        if rate.chroma_444() {
            apply_chroma_444(config, codec);
        }
    }
}

/// 4:4:4 of `codec` (P9): HEVC Main 4:4:4 (profile FREXT, `chromaFormatIDC`
/// 3); `false` for the codecs without a 4:4:4 profile on the wire (H.264:
/// `H264Profile` has no High 4:4:4; AV1: never in mode „Scharf“), which then
/// stay 4:2:0.
pub(in crate::screen::os) fn apply_chroma_444(config: &mut NvEncConfig, codec: CodecKind) -> bool {
    if codec != CodecKind::Hevc {
        return false;
    }
    config.profile_guid = NV_ENC_HEVC_PROFILE_FREXT_GUID;
    set_hevc_chroma(config, 3);
    true
}

/// Applies B-21 to an H.264 preset configuration for `rate`.
pub(in crate::screen::os) fn apply_settings(config: &mut NvEncConfig, rate: &impl NvRate) {
    apply_common(config, rate);
    config.profile_guid = NV_ENC_H264_PROFILE_HIGH_GUID;
    let h264 = &mut config.h264;
    h264.flags |= H264_REPEAT_SPSPPS;
    h264.idr_period = NVENC_INFINITE_GOPLENGTH;
    let vui = &mut h264.vui;
    vui.video_signal_type_present_flag = 1;
    vui.video_format = 5;
    vui.video_full_range_flag = 0;
    vui.colour_description_present_flag = 1;
    vui.colour_primaries = VUI_BT709;
    vui.transfer_characteristics = VUI_BT709;
    vui.colour_matrix = VUI_BT709;
}

/// The codec-independent part of B-21: GOP, no B pictures, CBR of one frame.
fn apply_common(config: &mut NvEncConfig, rate: &impl NvRate) {
    let (bits, buffer, initial) = rate.cbr();
    config.version = NV_ENC_CONFIG_VER;
    config.gop_length = NVENC_INFINITE_GOPLENGTH;
    config.frame_interval_p = 1;
    let rc = &mut config.rc_params;
    rc.version = NV_ENC_RC_PARAMS_VER;
    rc.rate_control_mode = NV_ENC_PARAMS_RC_CBR;
    rc.average_bit_rate = bits;
    rc.max_bit_rate = bits;
    rc.vbv_buffer_size = buffer;
    rc.vbv_initial_delay = initial;
    rc.flags |= RC_ZERO_REORDER_DELAY;
    rc.flags &= !RC_ENABLE_LOOKAHEAD;
    rc.lookahead_depth = 0;
}

/// Sunshine's additions to B-21 for H.264 (see the module header).
pub(in crate::screen::os) fn apply_reference_settings(
    config: &mut NvEncConfig,
    invalidation: bool,
) {
    config.rc_params.low_delay_key_frame_scale = 1;
    if invalidation {
        config.h264.max_num_ref_frames = RFI_DPB;
        config.h264.num_ref_l0 = NV_ENC_NUM_REF_FRAMES_1;
    }
}

/// `apply_reference_settings` for `codec` (HEVC and AV1: their members).
pub(in crate::screen::os) fn apply_codec_references(
    config: &mut NvEncConfig,
    codec: CodecKind,
    invalidation: bool,
) {
    if codec == CodecKind::H264 {
        apply_reference_settings(config, invalidation);
    } else {
        config.rc_params.low_delay_key_frame_scale = 1;
        if invalidation {
            apply_union_references(config, codec);
        }
    }
}

#[cfg(test)]
mod chroma_444_tests {
    use super::*;
    use crate::screen::os::nvenc_shared::abi::Zeroed;
    use crate::screen::os::nvenc_shared::codecs::hevc_view;

    struct Rate(bool);
    impl NvRate for Rate {
        fn cbr(&self) -> (u32, u32, u32) {
            (20_000_000, 333_334, 333_334)
        }
        fn chroma_444(&self) -> bool {
            self.0
        }
    }

    #[test]
    fn sharp_mode_task_nvenc_hevc_444_profile() {
        let mut plain = NvEncConfig::zeroed();
        apply_codec_settings(&mut plain, &Rate(false), CodecKind::Hevc);
        assert_ne!(plain.profile_guid, NV_ENC_HEVC_PROFILE_FREXT_GUID);
        assert_eq!((hevc_view(&mut plain).flags >> 9) & 0b11, 1, "4:2:0");
        let mut sharp = NvEncConfig::zeroed();
        apply_codec_settings(&mut sharp, &Rate(true), CodecKind::Hevc);
        assert_eq!(sharp.profile_guid, NV_ENC_HEVC_PROFILE_FREXT_GUID);
        let flags = hevc_view(&mut sharp).flags;
        assert_eq!((flags >> 9) & 0b11, 3, "4:4:4");
        assert_eq!(flags & (0b111 << 11), 0, "8 bit");
        assert_eq!(flags & (1 << 7), 1 << 7, "parameter sets at every IDR");
        // H.264 and AV1 have no 4:4:4 profile in this mode.
        let mut h264 = NvEncConfig::zeroed();
        assert!(!apply_chroma_444(&mut h264, CodecKind::H264));
        assert!(!apply_chroma_444(&mut h264, CodecKind::Av1));
        apply_codec_settings(&mut h264, &Rate(true), CodecKind::H264);
        assert_ne!(h264.profile_guid, NV_ENC_HEVC_PROFILE_FREXT_GUID);
    }
}
