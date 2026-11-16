//! HEVC parameter buffers of libva (plan 2026-11-14 „Bildkette“ BK4b):
//! `VAPictureHEVC` and the `VA_PICTURE_HEVC_*` flags of `va/va.h`, the
//! encoder's `VAEncSequenceParameterBufferHEVC`,
//! `VAEncPictureParameterBufferHEVC` and `VAEncSliceParameterBufferHEVC`
//! of `va/va_enc_hevc.h`, the decoder's `VAPictureParameterBufferHEVC` and
//! `VASliceParameterBufferHEVC` of `va/va_dec_hevc.h`, all of libva 2.20.0
//! (tag `2.20.0` of github.com/intel/libva, loaded through the jsDelivr
//! mirror and read on 2026-11-14; `VA_PADDING_LOW` 4, `_MEDIUM` 8,
//! `_HIGH` 16 of `va.h`). Field order, types and padding are copied from
//! those headers; the sizes and offsets in the comments were counted from
//! them for LP64 and are asserted by
//! `bildkette_task_linux_hw_hevc_struct_sizes`. Bit-field words are plain
//! `u32` with the bit positions of the header (System V: first member in
//! the least significant bit, assumption A-BK4B-BITS).
// Every field of these C layouts exists for its offset, read or not.
#![allow(dead_code)]
use super::vaapi_ffi::{VaId, VA_INVALID_ID};

/// `VA_PICTURE_HEVC_*` (va.h).
pub(super) const VA_PICTURE_HEVC_INVALID: u32 = 0x01;
pub(super) const VA_PICTURE_HEVC_LONG_TERM_REFERENCE: u32 = 0x08;
pub(super) const VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE: u32 = 0x10;
pub(super) const VA_PICTURE_HEVC_RPS_ST_CURR_AFTER: u32 = 0x20;
pub(super) const VA_PICTURE_HEVC_RPS_LT_CURR: u32 = 0x40;
/// An unused entry of `RefPicList` (va_dec_hevc.h: "0xFF indicates invalid").
pub(super) const HEVC_REF_INVALID: u8 = 0xff;

/// `VAPictureHEVC` (va.h, 28 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VaPictureHevc {
    pub picture_id: VaId,
    pub pic_order_cnt: i32,
    pub flags: u32,
    pub va_reserved: [u32; 4],
}

impl VaPictureHevc {
    /// An unused entry: `VA_INVALID_SURFACE` with `VA_PICTURE_HEVC_INVALID`.
    pub(super) fn invalid() -> Self {
        Self {
            picture_id: VA_INVALID_ID,
            flags: VA_PICTURE_HEVC_INVALID,
            ..Self::default()
        }
    }
}

/// `VAEncSequenceParameterBufferHEVC` (va_enc_hevc.h, 116 bytes).
/// `seq_fields`: chroma_format_idc 0–1, separate_colour_plane_flag 2,
/// bit_depth_luma_minus8 3–5, bit_depth_chroma_minus8 6–8,
/// scaling_list_enabled_flag 9, strong_intra_smoothing_enabled_flag 10,
/// amp_enabled_flag 11, sample_adaptive_offset_enabled_flag 12,
/// pcm_enabled_flag 13, pcm_loop_filter_disabled_flag 14,
/// sps_temporal_mvp_enabled_flag 15, low_delay_seq 16, hierachical_flag 17.
/// `vui_fields`: aspect_ratio_info_present_flag 0,
/// neutral_chroma_indication_flag 1, field_seq_flag 2,
/// vui_timing_info_present_flag 3, bitstream_restriction_flag 4,
/// tiles_fixed_structure_flag 5, motion_vectors_over_pic_boundaries_flag 6,
/// restricted_ref_pic_lists_flag 7, log2_max_mv_length_horizontal 8–12,
/// log2_max_mv_length_vertical 13–17.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncSequenceHevc {
    pub general_profile_idc: u8,
    pub general_level_idc: u8,
    pub general_tier_flag: u8,
    pub intra_period: u32,
    pub intra_idr_period: u32,
    pub ip_period: u32,
    pub bits_per_second: u32,
    pub pic_width_in_luma_samples: u16,
    pub pic_height_in_luma_samples: u16,
    pub seq_fields: u32,
    pub log2_min_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_luma_coding_block_size: u8,
    pub log2_min_transform_block_size_minus2: u8,
    pub log2_diff_max_min_transform_block_size: u8,
    pub max_transform_hierarchy_depth_inter: u8,
    pub max_transform_hierarchy_depth_intra: u8,
    pub pcm_sample_bit_depth_luma_minus1: u32,
    pub pcm_sample_bit_depth_chroma_minus1: u32,
    pub log2_min_pcm_luma_coding_block_size_minus3: u32,
    pub log2_max_pcm_luma_coding_block_size_minus3: u32,
    pub vui_parameters_present_flag: u8,
    pub vui_fields: u32,
    pub aspect_ratio_idc: u8,
    pub sar_width: u32,
    pub sar_height: u32,
    pub vui_num_units_in_tick: u32,
    pub vui_time_scale: u32,
    pub min_spatial_segmentation_idc: u16,
    pub max_bytes_per_pic_denom: u8,
    pub max_bits_per_min_cu_denom: u8,
    pub scc_fields: u32,
    pub va_reserved: [u32; 7],
}

/// `VAEncPictureParameterBufferHEVC` (va_enc_hevc.h, 576 bytes).
/// `pic_fields`: idr_pic_flag 0, coding_type 1–3 (I 1, P 2, B 3),
/// reference_pic_flag 4, dependent_slice_segments_enabled_flag 5,
/// sign_data_hiding_enabled_flag 6, constrained_intra_pred_flag 7,
/// transform_skip_enabled_flag 8, cu_qp_delta_enabled_flag 9,
/// weighted_pred_flag 10, weighted_bipred_flag 11,
/// transquant_bypass_enabled_flag 12, tiles_enabled_flag 13,
/// entropy_coding_sync_enabled_flag 14,
/// loop_filter_across_tiles_enabled_flag 15,
/// pps_loop_filter_across_slices_enabled_flag 16,
/// scaling_list_data_present_flag 17, screen_content_flag 18,
/// enable_gpu_weighted_prediction 19, no_output_of_prior_pics_flag 20.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncPictureHevc {
    pub decoded_curr_pic: VaPictureHevc,
    pub reference_frames: [VaPictureHevc; 15],
    pub coded_buf: VaId,
    pub collocated_ref_pic_index: u8,
    pub last_picture: u8,
    pub pic_init_qp: u8,
    pub diff_cu_qp_delta_depth: u8,
    pub pps_cb_qp_offset: i8,
    pub pps_cr_qp_offset: i8,
    pub num_tile_columns_minus1: u8,
    pub num_tile_rows_minus1: u8,
    pub column_width_minus1: [u8; 19],
    pub row_height_minus1: [u8; 21],
    pub log2_parallel_merge_level_minus2: u8,
    pub ctu_max_bitsize_allowed: u8,
    pub num_ref_idx_l0_default_active_minus1: u8,
    pub num_ref_idx_l1_default_active_minus1: u8,
    pub slice_pic_parameter_set_id: u8,
    pub nal_unit_type: u8,
    pub pic_fields: u32,
    pub hierarchical_level_plus1: u8,
    pub va_byte_reserved: u8,
    pub scc_fields: u16,
    pub va_reserved: [u32; 15],
}

/// `VAEncSliceParameterBufferHEVC` (va_enc_hevc.h, 1076 bytes).
/// `slice_fields`: last_slice_of_pic_flag 0, dependent_slice_segment_flag
/// 1, colour_plane_id 2–3, slice_temporal_mvp_enabled_flag 4,
/// slice_sao_luma_flag 5, slice_sao_chroma_flag 6,
/// num_ref_idx_active_override_flag 7, mvd_l1_zero_flag 8,
/// cabac_init_flag 9, slice_deblocking_filter_disabled_flag 10–11 (two
/// bits in the header), slice_loop_filter_across_slices_enabled_flag 12,
/// collocated_from_l0_flag 13.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaEncSliceHevc {
    pub slice_segment_address: u32,
    pub num_ctu_in_slice: u32,
    pub slice_type: u8,
    pub slice_pic_parameter_set_id: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub ref_pic_list0: [VaPictureHevc; 15],
    pub ref_pic_list1: [VaPictureHevc; 15],
    pub luma_log2_weight_denom: u8,
    pub delta_chroma_log2_weight_denom: i8,
    pub delta_luma_weight_l0: [i8; 15],
    pub luma_offset_l0: [i8; 15],
    pub delta_chroma_weight_l0: [[i8; 2]; 15],
    pub chroma_offset_l0: [[i8; 2]; 15],
    pub delta_luma_weight_l1: [i8; 15],
    pub luma_offset_l1: [i8; 15],
    pub delta_chroma_weight_l1: [[i8; 2]; 15],
    pub chroma_offset_l1: [[i8; 2]; 15],
    pub max_num_merge_cand: u8,
    pub slice_qp_delta: i8,
    pub slice_cb_qp_offset: i8,
    pub slice_cr_qp_offset: i8,
    pub slice_beta_offset_div2: i8,
    pub slice_tc_offset_div2: i8,
    pub slice_fields: u32,
    pub pred_weight_table_bit_offset: u32,
    pub pred_weight_table_bit_length: u32,
    pub va_reserved: [u32; 6],
}

/// `VAPictureParameterBufferHEVC` (va_dec_hevc.h, 604 bytes).
/// `pic_fields`: chroma_format_idc 0–1, separate_colour_plane_flag 2,
/// pcm_enabled_flag 3, scaling_list_enabled_flag 4,
/// transform_skip_enabled_flag 5, amp_enabled_flag 6,
/// strong_intra_smoothing_enabled_flag 7, sign_data_hiding_enabled_flag 8,
/// constrained_intra_pred_flag 9, cu_qp_delta_enabled_flag 10,
/// weighted_pred_flag 11, weighted_bipred_flag 12,
/// transquant_bypass_enabled_flag 13, tiles_enabled_flag 14,
/// entropy_coding_sync_enabled_flag 15,
/// pps_loop_filter_across_slices_enabled_flag 16,
/// loop_filter_across_tiles_enabled_flag 17,
/// pcm_loop_filter_disabled_flag 18, NoPicReorderingFlag 19,
/// NoBiPredFlag 20. `slice_parsing_fields`:
/// lists_modification_present_flag 0, long_term_ref_pics_present_flag 1,
/// sps_temporal_mvp_enabled_flag 2, cabac_init_present_flag 3,
/// output_flag_present_flag 4, dependent_slice_segments_enabled_flag 5,
/// pps_slice_chroma_qp_offsets_present_flag 6,
/// sample_adaptive_offset_enabled_flag 7,
/// deblocking_filter_override_enabled_flag 8,
/// pps_disable_deblocking_filter_flag 9,
/// slice_segment_header_extension_present_flag 10, RapPicFlag 11,
/// IdrPicFlag 12, IntraPicFlag 13.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaPictureParameterHevc {
    pub curr_pic: VaPictureHevc,
    pub reference_frames: [VaPictureHevc; 15],
    pub pic_width_in_luma_samples: u16,
    pub pic_height_in_luma_samples: u16,
    pub pic_fields: u32,
    pub sps_max_dec_pic_buffering_minus1: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub pcm_sample_bit_depth_luma_minus1: u8,
    pub pcm_sample_bit_depth_chroma_minus1: u8,
    pub log2_min_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_luma_coding_block_size: u8,
    pub log2_min_transform_block_size_minus2: u8,
    pub log2_diff_max_min_transform_block_size: u8,
    pub log2_min_pcm_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_pcm_luma_coding_block_size: u8,
    pub max_transform_hierarchy_depth_intra: u8,
    pub max_transform_hierarchy_depth_inter: u8,
    pub init_qp_minus26: i8,
    pub diff_cu_qp_delta_depth: u8,
    pub pps_cb_qp_offset: i8,
    pub pps_cr_qp_offset: i8,
    pub log2_parallel_merge_level_minus2: u8,
    pub num_tile_columns_minus1: u8,
    pub num_tile_rows_minus1: u8,
    pub column_width_minus1: [u16; 19],
    pub row_height_minus1: [u16; 21],
    pub slice_parsing_fields: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u8,
    pub num_short_term_ref_pic_sets: u8,
    pub num_long_term_ref_pic_sps: u8,
    pub num_ref_idx_l0_default_active_minus1: u8,
    pub num_ref_idx_l1_default_active_minus1: u8,
    pub pps_beta_offset_div2: i8,
    pub pps_tc_offset_div2: i8,
    pub num_extra_slice_header_bits: u8,
    pub st_rps_bits: u32,
    pub va_reserved: [u32; 8],
}

/// `VASliceParameterBufferHEVC` (va_dec_hevc.h, 264 bytes).
/// `long_slice_flags`: LastSliceOfPic 0, dependent_slice_segment_flag 1,
/// slice_type 2–3, color_plane_id 4–5, slice_sao_luma_flag 6,
/// slice_sao_chroma_flag 7, mvd_l1_zero_flag 8, cabac_init_flag 9,
/// slice_temporal_mvp_enabled_flag 10,
/// slice_deblocking_filter_disabled_flag 11, collocated_from_l0_flag 12,
/// slice_loop_filter_across_slices_enabled_flag 13.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VaSliceParameterHevc {
    pub slice_data_size: u32,
    pub slice_data_offset: u32,
    pub slice_data_flag: u32,
    /// Bytes from the NAL header to `slice_data()` without emulation
    /// prevention bytes; the data buffer keeps them (va_dec_hevc.h).
    pub slice_data_byte_offset: u32,
    pub slice_segment_address: u32,
    pub ref_pic_list: [[u8; 15]; 2],
    pub long_slice_flags: u32,
    pub collocated_ref_idx: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub slice_qp_delta: i8,
    pub slice_cb_qp_offset: i8,
    pub slice_cr_qp_offset: i8,
    pub slice_beta_offset_div2: i8,
    pub slice_tc_offset_div2: i8,
    pub luma_log2_weight_denom: u8,
    pub delta_chroma_log2_weight_denom: i8,
    pub delta_luma_weight_l0: [i8; 15],
    pub luma_offset_l0: [i8; 15],
    pub delta_chroma_weight_l0: [[i8; 2]; 15],
    pub chroma_offset_l0: [[i8; 2]; 15],
    pub delta_luma_weight_l1: [i8; 15],
    pub luma_offset_l1: [i8; 15],
    pub delta_chroma_weight_l1: [[i8; 2]; 15],
    pub chroma_offset_l1: [[i8; 2]; 15],
    pub five_minus_max_num_merge_cand: u8,
    pub num_entry_point_offsets: u16,
    pub entry_offset_to_subset_array: u16,
    pub slice_data_num_emu_prevn_bytes: u16,
    pub va_reserved: [u32; 2],
}

/// All-zero is a valid value of these plain C structs (integers and arrays
/// of integers only); this gives them that start.
pub(super) trait Zeroed: Copy + Sized {
    fn zeroed() -> Self {
        // SAFETY: implemented only for the `repr(C)` structs above, which
        // contain integers and arrays of integers alone.
        unsafe { std::mem::zeroed() }
    }
}
impl Zeroed for VaEncSequenceHevc {}
impl Zeroed for VaEncPictureHevc {}
impl Zeroed for VaEncSliceHevc {}
impl Zeroed for VaPictureParameterHevc {}
impl Zeroed for VaSliceParameterHevc {}
