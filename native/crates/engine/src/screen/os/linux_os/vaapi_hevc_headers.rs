//! The HEVC stream of the VA-API encoder (plan 2026-11-14 „Bildkette“
//! BK4b, B-21 as BK4's H.264): Main profile, 8-bit 4:2:0, one slice, P
//! pictures with the previous picture as their only reference (as B slices
//! with both lists equal where the driver demands both lists filled, the
//! "low-delay B" of `VA_PREDICTION_DIRECTION_BI_NOT_EMPTY`), an IDR only
//! for the first picture and on request, picture order = decoding order.
//! The program writes VPS, SPS and PPS (ITU-T H.265 (08/2021) 7.3.2.1,
//! 7.3.2.2, 7.3.2.3, profile_tier_level 7.3.3, VUI E.2.1 with BT.709
//! limited range and `max_dec_pic_buffering` 2) and, where the driver
//! takes packed slice headers, the slice segment header (7.3.6.1 with the
//! short-term reference picture set of 7.3.7 in the slice);
//! `vaapi_hevc_params.rs` maps the same values into the libva 2.20.0
//! buffers. Choices follow FFmpeg `libavcodec/vaapi_encode_h265.c`
//! (parameter sets in one packed sequence header, RPS in the slice, P as B
//! for GPB drivers) as known to the author (assumption A-VA-HEVC-FFMPEG).
use super::h264_bits::{annex_b_unit, BitWriter};
use super::vaapi_ffi::{
    VA_ATTRIB_NOT_SUPPORTED, VA_FEATURE_REQUIRED, VA_FEATURE_SUPPORTED,
    VA_PREDICTION_DIRECTION_BI_NOT_EMPTY,
};
pub(super) use super::vaapi_hevc_params::HevcPicture;
use crate::screen::core::pixel_convert::FrameSize;

/// NAL unit types (H.265 Table 7-1).
pub(super) const NAL_TRAIL_R: u8 = 1;
pub(super) const NAL_IDR_W_RADL: u8 = 19;
pub(super) const NAL_VPS: u8 = 32;
pub(super) const NAL_SPS: u8 = 33;
pub(super) const NAL_PPS: u8 = 34;
/// `slice_type` (Table 7-7).
pub(super) const SLICE_B: u8 = 0;
pub(super) const SLICE_P: u8 = 1;
pub(super) const SLICE_I: u8 = 2;
/// `log2_max_pic_order_cnt_lsb_minus4`: the POC LSBs have 16 bits.
pub(super) const LOG2_MAX_POC_LSB_MINUS4: u32 = 12;
pub(super) const PIC_INIT_QP: u8 = 26;
/// Merge candidates (`five_minus_max_num_merge_cand` 0).
pub(super) const MAX_MERGE_CAND: u8 = 5;

/// Block geometry of one encoder: CTB, coding and transform block sizes
/// from `VAConfigAttribEncHEVCBlockSizes`, or FFmpeg's defaults without it
/// (CTB 32, minimum CB 16, transform 4…32, depth 3; assumption
/// A-VA-HEVC-BLOCKS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HevcLayout {
    pub size: FrameSize,
    pub ctb_log2: u32,
    pub min_cb_log2: u32,
    pub min_tb_log2: u32,
    pub max_tb_log2: u32,
    pub depth_inter: u32,
    pub depth_intra: u32,
}

impl HevcLayout {
    pub(super) fn new(size: FrameSize, blocks: u32) -> Self {
        if blocks == VA_ATTRIB_NOT_SUPPORTED || (blocks & 3) == 0 {
            return Self {
                size,
                ctb_log2: 5,
                min_cb_log2: 4,
                min_tb_log2: 2,
                max_tb_log2: 5,
                depth_inter: 3,
                depth_intra: 3,
            };
        }
        // `VAConfigAttribValEncHEVCBlockSizes` (va_enc_hevc.h): two bits
        // each from bit 0: max CTB −3, min CTB −3, min CB −3, max TB −2,
        // min TB −2, max/min depth inter, max/min depth intra.
        let field = |shift: u32| (blocks >> shift) & 3;
        let ctb_log2 = field(0) + 3;
        let min_cb_log2 = (field(4) + 3).min(ctb_log2);
        // MinTbLog2SizeY < MinCbLog2SizeY (H.265 7.4.3.2.1).
        let min_tb_log2 = (field(8) + 2).min(min_cb_log2 - 1);
        let max_tb_log2 = (field(6) + 2).clamp(min_tb_log2, ctb_log2.min(5));
        let max_depth = ctb_log2 - min_tb_log2;
        Self {
            size,
            ctb_log2,
            min_cb_log2,
            min_tb_log2,
            max_tb_log2,
            depth_inter: field(10).min(max_depth),
            depth_intra: field(14).min(max_depth),
        }
    }

    fn align(value: u32, log2: u32) -> u32 {
        value.div_ceil(1 << log2) << log2
    }

    /// `pic_width/height_in_luma_samples`: whole minimum coding blocks.
    pub(super) fn coded(&self) -> FrameSize {
        FrameSize::new(
            Self::align(self.size.width, self.min_cb_log2),
            Self::align(self.size.height, self.min_cb_log2),
        )
    }

    /// The surface size: whole CTBs (assumption A-VA-HEVC-SURFACE).
    pub(super) fn surface(&self) -> FrameSize {
        FrameSize::new(
            Self::align(self.size.width, self.ctb_log2),
            Self::align(self.size.height, self.ctb_log2),
        )
    }

    /// `PicSizeInCtbsY` (7.4.3.2.1).
    pub(super) fn ctus(&self) -> u32 {
        let coded = self.coded();
        coded.width.div_ceil(1 << self.ctb_log2) * coded.height.div_ceil(1 << self.ctb_log2)
    }

    /// Conformance window right and bottom offset in chroma samples
    /// (SubWidthC = SubHeightC = 2).
    pub(super) fn crop(&self) -> (u32, u32) {
        let coded = self.coded();
        (
            (coded.width - self.size.width) / 2,
            (coded.height - self.size.height) / 2,
        )
    }
}

/// The coding tools of one encoder from `VAConfigAttribEncHEVCFeatures`
/// (two bits per tool, `VA_FEATURE_*`) and the prediction direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HevcTools {
    pub amp: bool,
    pub sao: bool,
    pub temporal_mvp: bool,
    pub strong_intra_smoothing: bool,
    pub sign_data_hiding: bool,
    pub constrained_intra_pred: bool,
    pub transform_skip: bool,
    pub cu_qp_delta: bool,
    /// P pictures as B slices with equal lists (low-delay B).
    pub gpb: bool,
}

impl HevcTools {
    /// Tools the driver supports are on (they only permit, the driver
    /// decides); tools that cost without benefit for the screen only where
    /// required. Without the attribute: AMP, SAO, temporal MVP and the
    /// QP delta of the rate control (assumption A-VA-HEVC-FEATURES).
    pub(super) fn new(features: u32, prediction: u32) -> Self {
        let gpb = prediction != VA_ATTRIB_NOT_SUPPORTED
            && prediction & VA_PREDICTION_DIRECTION_BI_NOT_EMPTY != 0;
        if features == VA_ATTRIB_NOT_SUPPORTED {
            return Self {
                amp: true,
                sao: true,
                temporal_mvp: true,
                strong_intra_smoothing: false,
                sign_data_hiding: false,
                constrained_intra_pred: false,
                transform_skip: false,
                cu_qp_delta: true,
                gpb,
            };
        }
        let field = |shift: u32| (features >> shift) & 3;
        let on = |shift| field(shift) >= VA_FEATURE_SUPPORTED;
        let required = |shift| field(shift) == VA_FEATURE_REQUIRED;
        Self {
            amp: on(4),
            sao: on(6),
            temporal_mvp: on(10),
            strong_intra_smoothing: on(12),
            sign_data_hiding: required(16),
            constrained_intra_pred: required(18),
            transform_skip: on(20),
            cu_qp_delta: on(22),
            gpb,
        }
    }
}

/// The two-byte NAL header (7.3.1.2): layer 0, `TemporalId` 0.
fn nal(kind: u8, rbsp: &[u8]) -> Vec<u8> {
    annex_b_unit(&[kind << 1, 1], rbsp)
}

/// `profile_tier_level(1, 0)` (7.3.3): Main, Main tier, progressive frames.
fn profile_tier_level(w: &mut BitWriter, level_idc: u8) {
    w.bits(2, 0); // general_profile_space
    w.flag(false); // general_tier_flag
    w.bits(5, 1); // general_profile_idc: Main
                  // general_profile_compatibility_flag[1] (Main) and [2] (Main 10).
    w.bits(32, (1 << 30) | (1 << 29));
    w.flag(true); // general_progressive_source_flag
    w.flag(false); // general_interlaced_source_flag
    w.flag(false); // general_non_packed_constraint_flag
    w.flag(true); // general_frame_only_constraint_flag
    w.bits(32, 0); // general_reserved_zero_43bits (first 32)
    w.bits(11, 0); // … the other 11
    w.flag(false); // general_inbld_flag
    w.bits(8, u32::from(level_idc));
}

/// The VPS NAL unit (7.3.2.1): one layer, one sub-layer.
pub(super) fn vps(level_idc: u8) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(4, 0); // vps_video_parameter_set_id
    w.flag(true); // vps_base_layer_internal_flag
    w.flag(true); // vps_base_layer_available_flag
    w.bits(6, 0); // vps_max_layers_minus1
    w.bits(3, 0); // vps_max_sub_layers_minus1
    w.flag(true); // vps_temporal_id_nesting_flag
    w.bits(16, 0xffff); // vps_reserved_0xffff_16bits
    profile_tier_level(&mut w, level_idc);
    w.flag(true); // vps_sub_layer_ordering_info_present_flag
    w.ue(1); // vps_max_dec_pic_buffering_minus1
    w.ue(0); // vps_max_num_reorder_pics
    w.ue(0); // vps_max_latency_increase_plus1
    w.bits(6, 0); // vps_max_layer_id
    w.ue(0); // vps_num_layer_sets_minus1
    w.flag(false); // vps_timing_info_present_flag
    w.flag(false); // vps_extension_flag
    nal(NAL_VPS, &w.finish())
}

/// The SPS NAL unit (7.3.2.2) matching `sequence_params`.
pub(super) fn sps(layout: &HevcLayout, tools: &HevcTools, level_idc: u8) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(4, 0); // sps_video_parameter_set_id
    w.bits(3, 0); // sps_max_sub_layers_minus1
    w.flag(true); // sps_temporal_id_nesting_flag
    profile_tier_level(&mut w, level_idc);
    w.ue(0); // sps_seq_parameter_set_id
    w.ue(1); // chroma_format_idc: 4:2:0
    let coded = layout.coded();
    w.ue(coded.width);
    w.ue(coded.height);
    let (right, bottom) = layout.crop();
    w.flag(right != 0 || bottom != 0); // conformance_window_flag
    if right != 0 || bottom != 0 {
        w.ue(0);
        w.ue(right);
        w.ue(0);
        w.ue(bottom);
    }
    w.ue(0); // bit_depth_luma_minus8
    w.ue(0); // bit_depth_chroma_minus8
    w.ue(LOG2_MAX_POC_LSB_MINUS4);
    w.flag(true); // sps_sub_layer_ordering_info_present_flag
    w.ue(1); // sps_max_dec_pic_buffering_minus1: reference + current
    w.ue(0); // sps_max_num_reorder_pics
    w.ue(0); // sps_max_latency_increase_plus1
    w.ue(layout.min_cb_log2 - 3);
    w.ue(layout.ctb_log2 - layout.min_cb_log2);
    w.ue(layout.min_tb_log2 - 2);
    w.ue(layout.max_tb_log2 - layout.min_tb_log2);
    w.ue(layout.depth_inter);
    w.ue(layout.depth_intra);
    w.flag(false); // scaling_list_enabled_flag
    w.flag(tools.amp);
    w.flag(tools.sao);
    w.flag(false); // pcm_enabled_flag
    w.ue(0); // num_short_term_ref_pic_sets: the RPS is in the slice
    w.flag(false); // long_term_ref_pics_present_flag
    w.flag(tools.temporal_mvp);
    w.flag(tools.strong_intra_smoothing);
    w.flag(true); // vui_parameters_present_flag
    vui(&mut w);
    w.flag(false); // sps_extension_present_flag
    nal(NAL_SPS, &w.finish())
}

/// VUI (E.2.1): BT.709 primaries, transfer and matrix in limited range; no
/// timing (the frame rate follows the viewer).
fn vui(w: &mut BitWriter) {
    w.flag(false); // aspect_ratio_info_present_flag
    w.flag(false); // overscan_info_present_flag
    w.flag(true); // video_signal_type_present_flag
    w.bits(3, 5); // video_format: unspecified
    w.flag(false); // video_full_range_flag
    w.flag(true); // colour_description_present_flag
    w.bits(8, 1); // colour_primaries: BT.709
    w.bits(8, 1); // transfer_characteristics: BT.709
    w.bits(8, 1); // matrix_coeffs: BT.709
    w.flag(false); // chroma_loc_info_present_flag
    w.flag(false); // neutral_chroma_indication_flag
    w.flag(false); // field_seq_flag
    w.flag(false); // frame_field_info_present_flag
    w.flag(false); // default_display_window_flag
    w.flag(false); // vui_timing_info_present_flag
    w.flag(true); // bitstream_restriction_flag
    w.flag(false); // tiles_fixed_structure_flag
    w.flag(true); // motion_vectors_over_pic_boundaries_flag
    w.flag(true); // restricted_ref_pic_lists_flag
    w.ue(0); // min_spatial_segmentation_idc
    w.ue(2); // max_bytes_per_pic_denom
    w.ue(1); // max_bits_per_min_cu_denom
    w.ue(15); // log2_max_mv_length_horizontal
    w.ue(15); // log2_max_mv_length_vertical
}

/// The PPS NAL unit (7.3.2.3.1) matching `picture_params`.
pub(super) fn pps(tools: &HevcTools) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(0); // pps_pic_parameter_set_id
    w.ue(0); // pps_seq_parameter_set_id
    w.flag(false); // dependent_slice_segments_enabled_flag
    w.flag(false); // output_flag_present_flag
    w.bits(3, 0); // num_extra_slice_header_bits
    w.flag(tools.sign_data_hiding);
    w.flag(false); // cabac_init_present_flag
    w.ue(0); // num_ref_idx_l0_default_active_minus1
    w.ue(0); // num_ref_idx_l1_default_active_minus1
    w.se(i32::from(PIC_INIT_QP) - 26); // init_qp_minus26
    w.flag(tools.constrained_intra_pred);
    w.flag(tools.transform_skip);
    w.flag(tools.cu_qp_delta);
    if tools.cu_qp_delta {
        w.ue(0); // diff_cu_qp_delta_depth
    }
    w.se(0); // pps_cb_qp_offset
    w.se(0); // pps_cr_qp_offset
    w.flag(false); // pps_slice_chroma_qp_offsets_present_flag
    w.flag(false); // weighted_pred_flag
    w.flag(false); // weighted_bipred_flag
    w.flag(false); // transquant_bypass_enabled_flag
    w.flag(false); // tiles_enabled_flag
    w.flag(false); // entropy_coding_sync_enabled_flag
    w.flag(false); // pps_loop_filter_across_slices_enabled_flag
    w.flag(true); // deblocking_filter_control_present_flag
    w.flag(false); // deblocking_filter_override_enabled_flag
    w.flag(false); // pps_deblocking_filter_disabled_flag
    w.se(0); // pps_beta_offset_div2
    w.se(0); // pps_tc_offset_div2
    w.flag(false); // pps_scaling_list_data_present_flag
    w.flag(false); // lists_modification_present_flag
    w.ue(0); // log2_parallel_merge_level_minus2
    w.flag(false); // slice_segment_header_extension_present_flag
    w.flag(false); // pps_extension_present_flag
    nal(NAL_PPS, &w.finish())
}

/// The parameter sets of an IDR in one packed sequence header.
pub(super) fn parameter_sets(layout: &HevcLayout, tools: &HevcTools, level_idc: u8) -> Vec<u8> {
    let mut out = vps(level_idc);
    out.extend_from_slice(&sps(layout, tools, level_idc));
    out.extend_from_slice(&pps(tools));
    out
}

/// The slice segment header NAL unit (7.3.6.1) of `picture`, written
/// where the driver takes packed slice headers; ends with
/// `byte_alignment()`, the driver appends the slice data.
pub(super) fn slice_header(tools: &HevcTools, picture: &HevcPicture) -> Vec<u8> {
    let mut w = BitWriter::default();
    let slice_type = picture.slice_type(tools);
    w.flag(true); // first_slice_segment_in_pic_flag
    if picture.idr {
        w.flag(false); // no_output_of_prior_pics_flag
    }
    w.ue(0); // slice_pic_parameter_set_id
    w.ue(u32::from(slice_type));
    if !picture.idr {
        let lsb_bits = LOG2_MAX_POC_LSB_MINUS4 + 4;
        w.bits(lsb_bits, picture.poc & ((1 << lsb_bits) - 1));
        w.flag(false); // short_term_ref_pic_set_sps_flag
                       // st_ref_pic_set(0) (7.3.7): the previous picture, used.
        w.ue(1); // num_negative_pics
        w.ue(0); // num_positive_pics
        w.ue(0); // delta_poc_s0_minus1
        w.flag(true); // used_by_curr_pic_s0_flag
        if tools.temporal_mvp {
            w.flag(true); // slice_temporal_mvp_enabled_flag
        }
    }
    if tools.sao {
        w.flag(true); // slice_sao_luma_flag
        w.flag(true); // slice_sao_chroma_flag
    }
    if slice_type != SLICE_I {
        w.flag(false); // num_ref_idx_active_override_flag: one each
        if slice_type == SLICE_B {
            w.flag(false); // mvd_l1_zero_flag
        }
        if tools.temporal_mvp && slice_type == SLICE_B {
            w.flag(true); // collocated_from_l0_flag
        }
        w.ue(u32::from(5 - MAX_MERGE_CAND)); // five_minus_max_num_merge_cand
    }
    w.se(0); // slice_qp_delta (assumption A-VA-HEVC-SLICE)
    nal(picture.nal_type(), &w.finish())
}
