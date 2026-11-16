//! AV1 parameter buffers of libva (plan 2026-11-14 „Bildkette“ BK4b): the
//! encoder's `VAEncSequenceParameterBufferAV1`, `VAEncSegParamAV1`,
//! `VAEncWarpedMotionParamsAV1`, `VAEncPictureParameterBufferAV1` and
//! `VAEncTileGroupBufferAV1` of `va/va_enc_av1.h`, the decoder's
//! `VASegmentationStructAV1`, `VAFilmGrainStructAV1`,
//! `VAWarpedMotionParamsAV1`, `VADecPictureParameterBufferAV1` and
//! `VASliceParameterBufferAV1` of `va/va_dec_av1.h`, all of libva 2.20.0
//! (tag `2.20.0` of github.com/intel/libva, read on 2026-11-14). Field
//! order, types and padding are copied from those headers; sizes and
//! offsets in the comments were counted for LP64 and are asserted by
//! `bildkette_task_linux_hw_av1_struct_sizes`. Bit-field words are plain
//! integers with the bit positions of the header (System V: first member
//! in the least significant bit, assumption A-BK4B-BITS).
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use std::ffi::c_int;

use super::vaapi_ffi::VaId;

/// `VAEncSequenceParameterBufferAV1` (va_enc_av1.h, 88 bytes).
/// `seq_fields`: still_picture 0, use_128x128_superblock 1,
/// enable_filter_intra 2, enable_intra_edge_filter 3,
/// enable_interintra_compound 4, enable_masked_compound 5,
/// enable_warped_motion 6, enable_dual_filter 7, enable_order_hint 8,
/// enable_jnt_comp 9, enable_ref_frame_mvs 10, enable_superres 11,
/// enable_cdef 12, enable_restoration 13, bit_depth_minus8 14–16,
/// subsampling_x 17, subsampling_y 18, mono_chrome 19.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncSequenceAv1 {
    pub seq_profile: u8,
    pub seq_level_idx: u8,
    pub seq_tier: u8,
    pub hierarchical_flag: u8,
    pub intra_period: u32,
    pub ip_period: u32,
    pub bits_per_second: u32,
    pub seq_fields: u32,
    pub order_hint_bits_minus_1: u8,
    pub va_reserved: [u32; 16],
}

/// `VAEncSegParamAV1` (va_enc_av1.h, 156 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncSegParamAv1 {
    pub seg_flags: u8,
    pub segment_number: u8,
    pub feature_data: [[i16; 8]; 8],
    pub feature_mask: [u8; 8],
    pub va_reserved: [u32; 4],
}

/// `VAEncWarpedMotionParamsAV1` and `VAWarpedMotionParamsAV1` (56 bytes;
/// `wmtype` is the C enum, an `int`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaWarpedMotionAv1 {
    pub wmtype: c_int,
    pub wmmat: [i32; 8],
    pub invalid: u8,
    pub pad: [u8; 3],
    pub va_reserved: [u32; 4],
}

impl VaWarpedMotionAv1 {
    /// The identity transformation (`gm_params` defaults of AV1 7.20:
    /// diagonal `1 << WARPEDMODEL_PREC_BITS`, 16).
    pub(super) fn identity() -> Self {
        Self {
            wmmat: [0, 0, 1 << 16, 0, 0, 1 << 16, 0, 0],
            ..Self::default()
        }
    }
}

/// `VAEncPictureParameterBufferAV1` (va_enc_av1.h, 1032 bytes).
/// `ref_frame_ctrl_l0/l1` (`VARefFrameCtrlAV1`): search_idx0..6 in three
/// bits each from bit 0, a reference name LAST 1 … ALTREF 7, 0 = none.
/// `picture_flags`: frame_type 0–1, error_resilient_mode 2,
/// disable_cdf_update 3, use_superres 4, allow_high_precision_mv 5,
/// use_ref_frame_mvs 6, disable_frame_end_update_cdf 7, reduced_tx_set 8,
/// enable_frame_obu 9, long_term_reference 10, disable_frame_recon 11,
/// allow_intrabc 12, palette_mode_enable 13. `loop_filter_flags`:
/// sharpness_level 0–2, mode_ref_delta_enabled 3, mode_ref_delta_update 4.
/// `mode_control_flags`: delta_q_present 0, delta_q_res 1–2,
/// delta_lf_present 3, delta_lf_res 4–5, delta_lf_multi 6, tx_mode 7–8,
/// reference_mode 9–10 (0 single, 1 compound, 2 select),
/// skip_mode_present 11. `tile_group_obu_hdr_info`: obu_extension_flag 0,
/// obu_has_size_field 1, temporal_id 2–4, spatial_id 5–6.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncPictureAv1 {
    pub frame_width_minus_1: u16,
    pub frame_height_minus_1: u16,
    pub reconstructed_frame: VaId,
    pub coded_buf: VaId,
    pub reference_frames: [VaId; 8],
    pub ref_frame_idx: [u8; 7],
    pub hierarchical_level_plus1: u8,
    pub primary_ref_frame: u8,
    pub order_hint: u8,
    pub refresh_frame_flags: u8,
    pub reserved8bits1: u8,
    pub ref_frame_ctrl_l0: u32,
    pub ref_frame_ctrl_l1: u32,
    pub picture_flags: u32,
    pub seg_id_block_size: u8,
    pub num_tile_groups_minus1: u8,
    pub temporal_id: u8,
    pub filter_level: [u8; 2],
    pub filter_level_u: u8,
    pub filter_level_v: u8,
    pub loop_filter_flags: u8,
    pub superres_scale_denominator: u8,
    pub interpolation_filter: u8,
    pub ref_deltas: [i8; 8],
    pub mode_deltas: [i8; 2],
    pub base_qindex: u8,
    pub y_dc_delta_q: i8,
    pub u_dc_delta_q: i8,
    pub u_ac_delta_q: i8,
    pub v_dc_delta_q: i8,
    pub v_ac_delta_q: i8,
    pub min_base_qindex: u8,
    pub max_base_qindex: u8,
    pub qmatrix_flags: u16,
    pub reserved16bits1: u16,
    pub mode_control_flags: u32,
    pub segments: VaEncSegParamAv1,
    pub tile_cols: u8,
    pub tile_rows: u8,
    pub reserved16bits2: u16,
    pub width_in_sbs_minus_1: [u16; 63],
    pub height_in_sbs_minus_1: [u16; 63],
    pub context_update_tile_id: u16,
    pub cdef_damping_minus_3: u8,
    pub cdef_bits: u8,
    pub cdef_y_strengths: [u8; 8],
    pub cdef_uv_strengths: [u8; 8],
    pub loop_restoration_flags: u16,
    pub wm: [VaWarpedMotionAv1; 7],
    pub bit_offset_qindex: u32,
    pub bit_offset_segmentation: u32,
    pub bit_offset_loopfilter_params: u32,
    pub bit_offset_cdef_params: u32,
    pub size_in_bits_cdef_params: u32,
    pub byte_offset_frame_hdr_obu_size: u32,
    pub size_in_bits_frame_hdr_obu: u32,
    pub tile_group_obu_hdr_info: u8,
    pub number_skip_frames: u8,
    pub reserved16bits3: u16,
    pub skip_frames_reduced_size: i32,
    pub va_reserved: [u32; 16],
}

/// `VAEncTileGroupBufferAV1` (va_enc_av1.h, 20 bytes), rendered as the
/// slice parameter buffer of an AV1 picture.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaEncTileGroupAv1 {
    pub tg_start: u8,
    pub tg_end: u8,
    pub pad: [u8; 2],
    pub va_reserved: [u32; 4],
}

/// `VASegmentationStructAV1` (va_dec_av1.h, 156 bytes);
/// `segment_info_fields`: enabled 0, update_map 1, temporal_update 2,
/// update_data 3. `feature_data` after Clip3 (header comment).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaSegmentationAv1 {
    pub segment_info_fields: u32,
    pub feature_data: [[i16; 8]; 8],
    pub feature_mask: [u8; 8],
    pub va_reserved: [u32; 4],
}

/// `VAFilmGrainStructAV1` (va_dec_av1.h, 176 bytes); this decoder refuses
/// film grain, so it stays all zero.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaFilmGrainAv1 {
    pub film_grain_info_fields: u32,
    pub grain_seed: u16,
    pub num_y_points: u8,
    pub point_y_value: [u8; 14],
    pub point_y_scaling: [u8; 14],
    pub num_cb_points: u8,
    pub point_cb_value: [u8; 10],
    pub point_cb_scaling: [u8; 10],
    pub num_cr_points: u8,
    pub point_cr_value: [u8; 10],
    pub point_cr_scaling: [u8; 10],
    pub ar_coeffs_y: [i8; 24],
    pub ar_coeffs_cb: [i8; 25],
    pub ar_coeffs_cr: [i8; 25],
    pub cb_mult: u8,
    pub cb_luma_mult: u8,
    pub cb_offset: u16,
    pub cr_mult: u8,
    pub cr_luma_mult: u8,
    pub cr_offset: u16,
    pub va_reserved: [u32; 4],
}

/// `VADecPictureParameterBufferAV1` (va_dec_av1.h, 1160 bytes, 8-aligned
/// by `anchor_frames_list`). `seq_info_fields`: still_picture 0,
/// use_128x128_superblock 1, enable_filter_intra 2,
/// enable_intra_edge_filter 3, enable_interintra_compound 4,
/// enable_masked_compound 5, enable_dual_filter 6, enable_order_hint 7,
/// enable_jnt_comp 8, enable_cdef 9, mono_chrome 10, color_range 11,
/// subsampling_x 12, subsampling_y 13, chroma_sample_position 14,
/// film_grain_params_present 15. `pic_info_fields`: frame_type 0–1,
/// show_frame 2, showable_frame 3, error_resilient_mode 4,
/// disable_cdf_update 5, allow_screen_content_tools 6, force_integer_mv 7,
/// allow_intrabc 8, use_superres 9, allow_high_precision_mv 10,
/// is_motion_mode_switchable 11, use_ref_frame_mvs 12,
/// disable_frame_end_update_cdf 13, uniform_tile_spacing_flag 14,
/// allow_warped_motion 15, large_scale_tile 16.
/// `loop_filter_info_fields`: sharpness_level 0–2, mode_ref_delta_enabled
/// 3, mode_ref_delta_update 4. `qmatrix_fields`: using_qmatrix 0, qm_y
/// 1–4, qm_u 5–8, qm_v 9–12. `mode_control_fields`: delta_q_present_flag
/// 0, log2_delta_q_res 1–2, delta_lf_present_flag 3, log2_delta_lf_res
/// 4–5, delta_lf_multi 6, tx_mode 7–8, reference_select 9,
/// reduced_tx_set_used 10, skip_mode_present 11.
/// `loop_restoration_fields`: yframe_restoration_type 0–1,
/// cbframe_restoration_type 2–3, crframe_restoration_type 4–5,
/// lr_unit_shift 6–7, lr_uv_shift 8.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaDecPictureAv1 {
    pub profile: u8,
    pub order_hint_bits_minus_1: u8,
    pub bit_depth_idx: u8,
    pub matrix_coefficients: u8,
    pub seq_info_fields: u32,
    pub current_frame: VaId,
    pub current_display_picture: VaId,
    pub anchor_frames_num: u8,
    pub anchor_frames_list: *mut VaId,
    pub frame_width_minus1: u16,
    pub frame_height_minus1: u16,
    pub output_frame_width_in_tiles_minus_1: u16,
    pub output_frame_height_in_tiles_minus_1: u16,
    pub ref_frame_map: [VaId; 8],
    pub ref_frame_idx: [u8; 7],
    pub primary_ref_frame: u8,
    pub order_hint: u8,
    pub seg_info: VaSegmentationAv1,
    pub film_grain_info: VaFilmGrainAv1,
    pub tile_cols: u8,
    pub tile_rows: u8,
    pub width_in_sbs_minus_1: [u16; 63],
    pub height_in_sbs_minus_1: [u16; 63],
    pub tile_count_minus_1: u16,
    pub context_update_tile_id: u16,
    pub pic_info_fields: u32,
    pub superres_scale_denominator: u8,
    pub interp_filter: u8,
    pub filter_level: [u8; 2],
    pub filter_level_u: u8,
    pub filter_level_v: u8,
    pub loop_filter_info_fields: u8,
    pub ref_deltas: [i8; 8],
    pub mode_deltas: [i8; 2],
    pub base_qindex: u8,
    pub y_dc_delta_q: i8,
    pub u_dc_delta_q: i8,
    pub u_ac_delta_q: i8,
    pub v_dc_delta_q: i8,
    pub v_ac_delta_q: i8,
    pub qmatrix_fields: u16,
    pub mode_control_fields: u32,
    pub cdef_damping_minus_3: u8,
    pub cdef_bits: u8,
    pub cdef_y_strengths: [u8; 8],
    pub cdef_uv_strengths: [u8; 8],
    pub loop_restoration_fields: u16,
    pub wm: [VaWarpedMotionAv1; 7],
    pub va_reserved: [u32; 8],
}

/// `VASliceParameterBufferAV1` (va_dec_av1.h, 40 bytes): one tile; the
/// offset is relative to the slice data buffer of its tile group.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VaSliceParameterAv1 {
    pub slice_data_size: u32,
    pub slice_data_offset: u32,
    pub slice_data_flag: u32,
    pub tile_row: u16,
    pub tile_column: u16,
    pub tg_start: u16,
    pub tg_end: u16,
    pub anchor_frame_idx: u8,
    pub tile_idx_in_tile_list: u16,
    pub va_reserved: [u32; 4],
}

/// All-zero is a valid value of these plain C structs (integers, arrays of
/// integers, and in `VaDecPictureAv1` one pointer, for which zero is the
/// null pointer libva reads as "no anchor frames").
pub(super) trait Zeroed: Copy + Sized {
    fn zeroed() -> Self {
        // SAFETY: implemented only for the `repr(C)` structs above, whose
        // members are integers, integer arrays and one nullable pointer.
        unsafe { std::mem::zeroed() }
    }
}
impl Zeroed for VaEncSequenceAv1 {}
impl Zeroed for VaEncSegParamAv1 {}
impl Zeroed for VaEncPictureAv1 {}
impl Zeroed for VaSegmentationAv1 {}
impl Zeroed for VaFilmGrainAv1 {}
impl Zeroed for VaDecPictureAv1 {}
