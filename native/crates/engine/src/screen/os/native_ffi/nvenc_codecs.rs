//! NVENC HEVC and AV1 of both platforms (BK6 on Linux, merge W1 on Windows;
//! one module since merge W2, plan 2026-11-14 „Bildkette“ B-23/B-30):
//! GUIDs, the views on the codec union, `apply_union` and the B-36 reference
//! settings of the HEVC and AV1 members; every value from `nvEncodeAPI.h` of
//! NVENC API 12.1 (nv-codec-headers tag `n12.1.14.0`); the layouts hold only
//! `u32` and pointers, so LP64 and LLP64 agree (assumption A-BK6-NV-LAYOUT,
//! asserted below at compile time). The factories stay platform code
//! (`linux_os/nvenc_codecs.rs`, `windows/vendor_codecs.rs`).
//!
//! Reference frames for B-36 as Sunshine `nvenc_base.cpp:284-298`, `:372`,
//! `:408`, `:449`: a DPB of 5 pictures for H.264 and HEVC, 8 for AV1, one
//! reference per picture (`numRefL0` / `numFwdRefs` = 1), only while the
//! encoder can invalidate references.
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use super::abi::{Guid, NvEncConfig, NvEncConfigH264, NvEncVui, Ptr};
use super::api::{NV_ENC_CODEC_H264_GUID, NV_ENC_NUM_REF_FRAMES_1, VUI_BT709};
use crate::screen::core::negotiate::CodecKind;

pub(in crate::screen::os) const NV_ENC_CODEC_HEVC_GUID: Guid = Guid {
    data1: 0x790c_dc88,
    data2: 0x4522,
    data3: 0x4d7b,
    data4: [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03],
};
pub(in crate::screen::os) const NV_ENC_CODEC_AV1_GUID: Guid = Guid {
    data1: 0x0a35_2289,
    data2: 0x0aa7,
    data3: 0x4759,
    data4: [0x86, 0x2d, 0x5d, 0x15, 0xcd, 0x16, 0xd2, 0x54],
};
pub(in crate::screen::os) const NV_ENC_HEVC_PROFILE_MAIN_GUID: Guid = Guid {
    data1: 0xb514_c39a,
    data2: 0xb55b,
    data3: 0x40fa,
    data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec],
};
pub(in crate::screen::os) const NV_ENC_AV1_PROFILE_MAIN_GUID: Guid = Guid {
    data1: 0x5f2a_39f5,
    data2: 0xf14e,
    data3: 0x4f95,
    data4: [0x9a, 0x9e, 0xb7, 0x6d, 0x56, 0x8f, 0xcf, 0x97],
};
const NV_ENC_LEVEL_AUTOSELECT: u32 = 0;
const NV_ENC_LEVEL_AV1_AUTOSELECT: u32 = 24;
/// `NV_ENC_CONFIG_HEVC` bit fields: `repeatSPSPPS` bit 7, `chromaFormatIDC`
/// bits 9–10, `pixelBitDepthMinus8` bits 11–13.
const HEVC_REPEAT_SPSPPS: u32 = 1 << 7;
const HEVC_CHROMA_MASK: u32 = 0b11 << 9;
const HEVC_DEPTH_MASK: u32 = 0b111 << 11;
/// `NV_ENC_CONFIG_AV1` bit fields: `outputAnnexBFormat` bit 0,
/// `enableFrameIdNumbers` bit 3, `disableSeqHdr` bit 4, `repeatSeqHdr` bit 5,
/// `chromaFormatIDC` bits 7–8, `inputPixelBitDepthMinus8` bits 12–14,
/// `pixelBitDepthMinus8` 15–17.
const AV1_ANNEX_B: u32 = 1;
const AV1_ENABLE_FRAME_ID: u32 = 1 << 3;
const AV1_DISABLE_SEQ_HDR: u32 = 1 << 4;
const AV1_REPEAT_SEQ_HDR: u32 = 1 << 5;
const AV1_CHROMA_MASK: u32 = 0b11 << 7;
const AV1_DEPTH_MASK: u32 = (0b111 << 12) | (0b111 << 15);

/// `NV_ENC_CONFIG_HEVC` (1560 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncConfigHevc {
    pub level: u32,
    pub tier: u32,
    pub min_cu_size: u32,
    pub max_cu_size: u32,
    pub flags: u32,
    pub idr_period: u32,
    pub intra_refresh_period: u32,
    pub intra_refresh_cnt: u32,
    pub max_num_ref_frames_in_dpb: u32,
    pub ltr_num_frames: u32,
    pub vps_id: u32,
    pub sps_id: u32,
    pub pps_id: u32,
    pub slice_mode: u32,
    pub slice_mode_data: u32,
    pub max_temporal_layers_minus1: u32,
    pub vui: NvEncVui,
    pub ltr_trust_mode: u32,
    pub use_bframes_as_ref: u32,
    pub num_ref_l0: u32,
    pub num_ref_l1: u32,
    pub reserved1: [u32; 214],
    pub reserved2: [Ptr; 64],
}

/// `NV_ENC_CONFIG_AV1` (1552 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct NvEncConfigAv1 {
    pub level: u32,
    pub tier: u32,
    pub min_part_size: u32,
    pub max_part_size: u32,
    pub flags: u32,
    pub idr_period: u32,
    pub intra_refresh_period: u32,
    pub intra_refresh_cnt: u32,
    pub max_num_ref_frames_in_dpb: u32,
    pub num_tile_columns: u32,
    pub num_tile_rows: u32,
    pub tile_widths: *mut u32,
    pub tile_heights: *mut u32,
    pub max_temporal_layers_minus1: u32,
    pub color_primaries: u32,
    pub transfer_characteristics: u32,
    pub matrix_coefficients: u32,
    pub color_range: u32,
    pub chroma_sample_position: u32,
    pub use_bframes_as_ref: u32,
    pub film_grain_params: Ptr,
    pub num_fwd_refs: u32,
    pub num_bwd_refs: u32,
    pub reserved1: [u32; 235],
    pub reserved2: [Ptr; 62],
}

const _: () = assert!(std::mem::size_of::<NvEncConfigHevc>() == 1560);
const _: () = assert!(std::mem::size_of::<NvEncConfigAv1>() == 1552);
const _: () = assert!(std::mem::size_of::<NvEncConfigH264>() >= 1560);

/// The encode GUID of `codec`; `None` for the tiles.
pub(in crate::screen::os) fn encode_guid(codec: CodecKind) -> Option<Guid> {
    match codec {
        CodecKind::H264 => Some(NV_ENC_CODEC_H264_GUID),
        CodecKind::Hevc => Some(NV_ENC_CODEC_HEVC_GUID),
        CodecKind::Av1 => Some(NV_ENC_CODEC_AV1_GUID),
        CodecKind::Tiles => None,
    }
}

/// The HEVC member of the codec union.
fn hevc(config: &mut NvEncConfig) -> &mut NvEncConfigHevc {
    let union: *mut NvEncConfigH264 = &mut config.h264;
    // SAFETY: the union area is `NvEncConfigH264` (1792 bytes, 8-aligned);
    // `NvEncConfigHevc` is smaller, as aligned, and all-zero is valid for it.
    unsafe { &mut *union.cast::<NvEncConfigHevc>() }
}

/// The AV1 member of the codec union.
fn av1(config: &mut NvEncConfig) -> &mut NvEncConfigAv1 {
    let union: *mut NvEncConfigH264 = &mut config.h264;
    // SAFETY: as `hevc`; `NvEncConfigAv1` is 1552 bytes, 8-aligned.
    unsafe { &mut *union.cast::<NvEncConfigAv1>() }
}

/// Writes the HEVC or AV1 part of B-21 into the preset `config`; the
/// common part (rate control, GOP, no B pictures) is `settings`'.
pub(in crate::screen::os) fn apply_union(
    config: &mut NvEncConfig,
    codec: CodecKind,
    idr_period: u32,
) {
    match codec {
        CodecKind::Hevc => {
            config.profile_guid = NV_ENC_HEVC_PROFILE_MAIN_GUID;
            let hevc = hevc(config);
            hevc.level = NV_ENC_LEVEL_AUTOSELECT;
            hevc.tier = 0;
            hevc.flags = (hevc.flags & !(HEVC_CHROMA_MASK | HEVC_DEPTH_MASK))
                | HEVC_REPEAT_SPSPPS
                | (1 << 9);
            hevc.idr_period = idr_period;
            let vui = &mut hevc.vui;
            vui.video_signal_type_present_flag = 1;
            vui.video_format = 5;
            vui.video_full_range_flag = 0;
            vui.colour_description_present_flag = 1;
            vui.colour_primaries = VUI_BT709;
            vui.transfer_characteristics = VUI_BT709;
            vui.colour_matrix = VUI_BT709;
        }
        CodecKind::Av1 => {
            config.profile_guid = NV_ENC_AV1_PROFILE_MAIN_GUID;
            let av1 = av1(config);
            av1.level = NV_ENC_LEVEL_AV1_AUTOSELECT;
            av1.tier = 0;
            av1.flags = (av1.flags
                & !(AV1_ANNEX_B
                    | AV1_ENABLE_FRAME_ID
                    | AV1_DISABLE_SEQ_HDR
                    | AV1_CHROMA_MASK
                    | AV1_DEPTH_MASK))
                | AV1_REPEAT_SEQ_HDR
                | (1 << 7);
            av1.idr_period = idr_period;
            av1.color_primaries = VUI_BT709;
            av1.transfer_characteristics = VUI_BT709;
            av1.matrix_coefficients = VUI_BT709;
            av1.color_range = 0;
        }
        CodecKind::H264 | CodecKind::Tiles => {}
    }
}

/// Sets `chromaFormatIDC` of the HEVC member to `idc` (1: 4:2:0, 3: 4:4:4;
/// `settings::apply_chroma_444`, plan 2026-11-17 P9).
pub(in crate::screen::os) fn set_hevc_chroma(config: &mut NvEncConfig, idc: u32) {
    let hevc = hevc(config);
    hevc.flags = (hevc.flags & !HEVC_CHROMA_MASK) | ((idc & 0b11) << 9);
}

/// DPB size with reference invalidation of `codec` (Sunshine: 5 for H.264
/// and HEVC, 8 for AV1).
pub(in crate::screen::os) fn rfi_dpb(codec: CodecKind) -> u32 {
    if codec == CodecKind::Av1 {
        8
    } else {
        5
    }
}

/// B-36 reference settings of the HEVC and AV1 members (H.264:
/// `settings::apply_reference_settings`).
pub(in crate::screen::os) fn apply_union_references(config: &mut NvEncConfig, codec: CodecKind) {
    let one = NV_ENC_NUM_REF_FRAMES_1;
    match codec {
        CodecKind::Hevc => {
            let hevc = hevc(config);
            hevc.max_num_ref_frames_in_dpb = rfi_dpb(codec);
            hevc.num_ref_l0 = one;
        }
        CodecKind::Av1 => {
            let av1 = av1(config);
            av1.max_num_ref_frames_in_dpb = rfi_dpb(codec);
            av1.num_fwd_refs = one;
        }
        CodecKind::H264 | CodecKind::Tiles => {}
    }
}

/// The view of the HEVC member (task tests).
#[cfg(test)]
pub(in crate::screen::os) fn hevc_view(config: &mut NvEncConfig) -> &mut NvEncConfigHevc {
    hevc(config)
}

/// The view of the AV1 member (task tests).
#[cfg(test)]
pub(in crate::screen::os) fn av1_view(config: &mut NvEncConfig) -> &mut NvEncConfigAv1 {
    av1(config)
}

/// The name of the NVENC factory of `codec` (`se screen codec`, the host
/// process's `refused` list keeps the codecs apart; as BK6 names them).
pub(in crate::screen::os) fn factory_name(codec: CodecKind) -> String {
    match codec {
        CodecKind::H264 => super::settings::NVENC_NAME.into(),
        other => format!("{}-{}", super::settings::NVENC_NAME, other.key()),
    }
}
