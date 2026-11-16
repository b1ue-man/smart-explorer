//! H.264 parameter buffers of libva (BK4): the encoder's sequence,
//! picture, slice, misc and packed-header buffers (`va/va_enc_h264.h`,
//! `va/va.h`) and the decoder's picture, slice and IQ-matrix buffers
//! (`va/va.h`), all of libva 2.20.0 (tag `2.20.0`, read on 2026-11-14).
//! Field order, types and padding are copied from those headers; the sizes
//! in the comments were counted from them for LP64 and are asserted by
//! `bildkette_task_linux_hw_va_struct_sizes`. Bit-field words are plain
//! `u32` with the bit positions of the header (System V: first member in
//! the least significant bit), set through the constants below.
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use super::vaapi_ffi::VaId;

/// `VA_PICTURE_H264_*` flags (va.h).
pub(super) const VA_PICTURE_H264_INVALID: u32 = 0x01;
pub(super) const VA_PICTURE_H264_SHORT_TERM_REFERENCE: u32 = 0x08;
pub(super) const VA_PICTURE_H264_LONG_TERM_REFERENCE: u32 = 0x10;
/// `VAEncMiscParameterType` (va.h).
pub(super) const VA_ENC_MISC_FRAME_RATE: u32 = 0;
pub(super) const VA_ENC_MISC_RATE_CONTROL: u32 = 1;
pub(super) const VA_ENC_MISC_HRD: u32 = 5;
/// `VAEncMiscParameterTypeQualityLevel` (va.h 2.20.0 line 2294).
pub(super) const VA_ENC_MISC_QUALITY_LEVEL: u32 = 6;
/// `VA_SLICE_DATA_FLAG_ALL` (va.h).
pub(super) const VA_SLICE_DATA_FLAG_ALL: u32 = 0;

/// `VAPictureH264` (36 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaPictureH264 {
    pub picture_id: VaId,
    pub frame_idx: u32,
    pub flags: u32,
    pub top_field_order_cnt: i32,
    pub bottom_field_order_cnt: i32,
    pub va_reserved: [u32; 4],
}

impl VaPictureH264 {
    /// An unused entry: `VA_INVALID_SURFACE` with `VA_PICTURE_H264_INVALID`.
    pub(super) fn invalid() -> Self {
        Self {
            picture_id: super::vaapi_ffi::VA_INVALID_ID,
            flags: VA_PICTURE_H264_INVALID,
            ..Self::default()
        }
    }
}

/// `VAEncSequenceParameterBufferH264` (va_enc_h264.h, 1132 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncSequenceH264 {
    pub seq_parameter_set_id: u8,
    pub level_idc: u8,
    pub intra_period: u32,
    pub intra_idr_period: u32,
    pub ip_period: u32,
    pub bits_per_second: u32,
    pub max_num_ref_frames: u32,
    pub picture_width_in_mbs: u16,
    pub picture_height_in_mbs: u16,
    /// `seq_fields`: chroma_format_idc 0–1, frame_mbs_only_flag 2,
    /// mb_adaptive_frame_field_flag 3, seq_scaling_matrix_present_flag 4,
    /// direct_8x8_inference_flag 5, log2_max_frame_num_minus4 6–9,
    /// pic_order_cnt_type 10–11, log2_max_pic_order_cnt_lsb_minus4 12–15,
    /// delta_pic_order_always_zero_flag 16.
    pub seq_fields: u32,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub num_ref_frames_in_pic_order_cnt_cycle: u8,
    pub offset_for_non_ref_pic: i32,
    pub offset_for_top_to_bottom_field: i32,
    pub offset_for_ref_frame: [i32; 256],
    pub frame_cropping_flag: u8,
    pub frame_crop_left_offset: u32,
    pub frame_crop_right_offset: u32,
    pub frame_crop_top_offset: u32,
    pub frame_crop_bottom_offset: u32,
    pub vui_parameters_present_flag: u8,
    /// `vui_fields`: aspect_ratio_info_present_flag 0,
    /// timing_info_present_flag 1, bitstream_restriction_flag 2,
    /// log2_max_mv_length_horizontal 3–7, log2_max_mv_length_vertical 8–12,
    /// fixed_frame_rate_flag 13, low_delay_hrd_flag 14,
    /// motion_vectors_over_pic_boundaries_flag 15.
    pub vui_fields: u32,
    pub aspect_ratio_idc: u8,
    pub sar_width: u32,
    pub sar_height: u32,
    pub num_units_in_tick: u32,
    pub time_scale: u32,
    pub va_reserved: [u32; 4],
}

/// `VAEncPictureParameterBufferH264` (va_enc_h264.h, 648 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncPictureH264 {
    pub curr_pic: VaPictureH264,
    pub reference_frames: [VaPictureH264; 16],
    pub coded_buf: VaId,
    pub pic_parameter_set_id: u8,
    pub seq_parameter_set_id: u8,
    pub last_picture: u8,
    pub frame_num: u16,
    pub pic_init_qp: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub chroma_qp_index_offset: i8,
    pub second_chroma_qp_index_offset: i8,
    /// `pic_fields`: idr_pic_flag 0, reference_pic_flag 1–2,
    /// entropy_coding_mode_flag 3, weighted_pred_flag 4,
    /// weighted_bipred_idc 5–6, constrained_intra_pred_flag 7,
    /// transform_8x8_mode_flag 8, deblocking_filter_control_present_flag 9,
    /// redundant_pic_cnt_present_flag 10, pic_order_present_flag 11,
    /// pic_scaling_matrix_present_flag 12.
    pub pic_fields: u32,
    pub va_reserved: [u32; 4],
}

/// `VAEncSliceParameterBufferH264` (va_enc_h264.h, 3140 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncSliceH264 {
    pub macroblock_address: u32,
    pub num_macroblocks: u32,
    pub macroblock_info: VaId,
    pub slice_type: u8,
    pub pic_parameter_set_id: u8,
    pub idr_pic_id: u16,
    pub pic_order_cnt_lsb: u16,
    pub delta_pic_order_cnt_bottom: i32,
    pub delta_pic_order_cnt: [i32; 2],
    pub direct_spatial_mv_pred_flag: u8,
    pub num_ref_idx_active_override_flag: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub ref_pic_list0: [VaPictureH264; 32],
    pub ref_pic_list1: [VaPictureH264; 32],
    pub luma_log2_weight_denom: u8,
    pub chroma_log2_weight_denom: u8,
    pub luma_weight_l0_flag: u8,
    pub luma_weight_l0: [i16; 32],
    pub luma_offset_l0: [i16; 32],
    pub chroma_weight_l0_flag: u8,
    pub chroma_weight_l0: [[i16; 2]; 32],
    pub chroma_offset_l0: [[i16; 2]; 32],
    pub luma_weight_l1_flag: u8,
    pub luma_weight_l1: [i16; 32],
    pub luma_offset_l1: [i16; 32],
    pub chroma_weight_l1_flag: u8,
    pub chroma_weight_l1: [[i16; 2]; 32],
    pub chroma_offset_l1: [[i16; 2]; 32],
    pub cabac_init_idc: u8,
    pub slice_qp_delta: i8,
    pub disable_deblocking_filter_idc: u8,
    pub slice_alpha_c0_offset_div2: i8,
    pub slice_beta_offset_div2: i8,
    pub va_reserved: [u32; 4],
}

/// `VAEncMiscParameterRateControl` (va.h, 60 bytes). `rc_flags`: reset 0,
/// disable_frame_skip 1, disable_bit_stuffing 2.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaEncMiscRateControl {
    pub bits_per_second: u32,
    pub target_percentage: u32,
    pub window_size: u32,
    pub initial_qp: u32,
    pub min_qp: u32,
    pub basic_unit_size: u32,
    pub rc_flags: u32,
    pub icq_quality_factor: u32,
    pub max_qp: u32,
    pub quality_factor: u32,
    pub target_frame_size: u32,
    pub va_reserved: [u32; 4],
}
pub(super) const RC_FLAG_RESET: u32 = 1;
pub(super) const RC_FLAG_DISABLE_FRAME_SKIP: u32 = 1 << 1;

/// `VAEncMiscParameterHRD` (va.h, 24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaEncMiscHrd {
    pub initial_buffer_fullness: u32,
    pub buffer_size: u32,
    pub va_reserved: [u32; 4],
}

/// `VAEncMiscParameterFrameRate` (va.h, 24 bytes); `framerate` is
/// numerator | denominator << 16.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaEncMiscFrameRate {
    pub framerate: u32,
    pub framerate_flags: u32,
    pub va_reserved: [u32; 4],
}

/// `VAEncMiscParameterBufferQualityLevel` (va.h 2.20.0 lines 2707-2715,
/// 20 bytes): "A lower value means higher quality, and a value of 1
/// represents the highest quality ... higher quality corresponds to lower
/// speed"; 0 is the driver's default. It holds for the coded sequence.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaEncMiscQualityLevel {
    pub quality_level: u32,
    pub va_reserved: [u32; 4],
}

/// `VAEncMiscParameterBuffer` followed by its payload: the header's
/// `uint32_t type; uint32_t data[];`, so the payload (only `u32` fields)
/// starts at offset 4.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct VaEncMisc<T: Copy> {
    pub kind: u32,
    pub data: T,
}

/// `VAEncPackedHeaderParameterBuffer` (va.h, 28 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaEncPackedHeaderParameter {
    pub kind: u32,
    pub bit_length: u32,
    pub has_emulation_bytes: u8,
    pub pad: [u8; 3],
    pub va_reserved: [u32; 4],
}

/// `VAPictureParameterBufferH264` (va.h, 672 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaPictureParameterH264 {
    pub curr_pic: VaPictureH264,
    pub reference_frames: [VaPictureH264; 16],
    pub picture_width_in_mbs_minus1: u16,
    pub picture_height_in_mbs_minus1: u16,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub num_ref_frames: u8,
    /// `seq_fields`: chroma_format_idc 0–1, residual_colour_transform_flag 2,
    /// gaps_in_frame_num_value_allowed_flag 3, frame_mbs_only_flag 4,
    /// mb_adaptive_frame_field_flag 5, direct_8x8_inference_flag 6,
    /// MinLumaBiPredSize8x8 7, log2_max_frame_num_minus4 8–11,
    /// pic_order_cnt_type 12–13, log2_max_pic_order_cnt_lsb_minus4 14–17,
    /// delta_pic_order_always_zero_flag 18.
    pub seq_fields: u32,
    pub num_slice_groups_minus1: u8,
    pub slice_group_map_type: u8,
    pub slice_group_change_rate_minus1: u16,
    pub pic_init_qp_minus26: i8,
    pub pic_init_qs_minus26: i8,
    pub chroma_qp_index_offset: i8,
    pub second_chroma_qp_index_offset: i8,
    /// `pic_fields`: entropy_coding_mode_flag 0, weighted_pred_flag 1,
    /// weighted_bipred_idc 2–3, transform_8x8_mode_flag 4, field_pic_flag 5,
    /// constrained_intra_pred_flag 6, pic_order_present_flag 7,
    /// deblocking_filter_control_present_flag 8,
    /// redundant_pic_cnt_present_flag 9, reference_pic_flag 10.
    pub pic_fields: u32,
    pub frame_num: u16,
    pub va_reserved: [u32; 8],
}

/// `VASliceParameterBufferH264` (va.h, 3128 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaSliceParameterH264 {
    pub slice_data_size: u32,
    pub slice_data_offset: u32,
    pub slice_data_flag: u32,
    pub slice_data_bit_offset: u16,
    pub first_mb_in_slice: u16,
    pub slice_type: u8,
    pub direct_spatial_mv_pred_flag: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub cabac_init_idc: u8,
    pub slice_qp_delta: i8,
    pub disable_deblocking_filter_idc: u8,
    pub slice_alpha_c0_offset_div2: i8,
    pub slice_beta_offset_div2: i8,
    pub ref_pic_list0: [VaPictureH264; 32],
    pub ref_pic_list1: [VaPictureH264; 32],
    pub luma_log2_weight_denom: u8,
    pub chroma_log2_weight_denom: u8,
    pub luma_weight_l0_flag: u8,
    pub luma_weight_l0: [i16; 32],
    pub luma_offset_l0: [i16; 32],
    pub chroma_weight_l0_flag: u8,
    pub chroma_weight_l0: [[i16; 2]; 32],
    pub chroma_offset_l0: [[i16; 2]; 32],
    pub luma_weight_l1_flag: u8,
    pub luma_weight_l1: [i16; 32],
    pub luma_offset_l1: [i16; 32],
    pub chroma_weight_l1_flag: u8,
    pub chroma_weight_l1: [[i16; 2]; 32],
    pub chroma_offset_l1: [[i16; 2]; 32],
    pub va_reserved: [u32; 4],
}

/// `VAIQMatrixBufferH264` (va.h, 240 bytes). The header calls the lists
/// "raster scan order"; FFmpeg's `vaapi_h264.c` and the Intel drivers pass
/// them in the bitstream's zig-zag order, which this decoder follows
/// (assumption A-VA-IQ in the BK4 report; flat lists are order-free).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaIqMatrixH264 {
    pub scaling_list4x4: [[u8; 16]; 6],
    pub scaling_list8x8: [[u8; 64]; 2],
    pub va_reserved: [u32; 4],
}

/// The parameter buffers are plain C data whose all-zero bit pattern is a
/// valid value (integers, arrays of integers); this gives them that start.
pub(super) trait Zeroed: Copy + Sized {
    fn zeroed() -> Self {
        // SAFETY: implemented only for the `repr(C)` structs above, which
        // contain integers and arrays of integers alone.
        unsafe { std::mem::zeroed() }
    }
}
impl Zeroed for VaEncSequenceH264 {}
impl Zeroed for VaEncPictureH264 {}
impl Zeroed for VaEncSliceH264 {}
impl Zeroed for VaPictureParameterH264 {}
impl Zeroed for VaSliceParameterH264 {}
impl Zeroed for VaIqMatrixH264 {}
