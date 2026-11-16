//! The H.264 stream layout of the VA-API encoder (BK4, B-21 analogously)
//! and its SPS and PPS (H.264 7.3.2.1.1, 7.3.2.2, E.1.1), written by this
//! program where the driver accepts packed headers, so the VUI carries
//! BT.709 limited range and `max_dec_frame_buffering = 1` (a viewer's
//! decoder shows every picture at once). One reference frame, P pictures
//! only, picture order count type 2 (output order = decoding order; FFmpeg
//! `vaapi_encode_h264.c` uses it without B frames), `frame_num` of 16 bits.
use super::h264_bits::{annex_b_nal, BitWriter, NAL_PPS, NAL_SPS};
use super::hw_encode_common::RateParams;
use super::vaapi_ffi::{VaId, VA_INVALID_ID};
use super::vaapi_h264_abi::{
    VaEncMiscFrameRate, VaEncMiscHrd, VaEncMiscRateControl, VaEncPictureH264, VaEncSequenceH264,
    VaEncSliceH264, VaPictureH264, Zeroed, RC_FLAG_DISABLE_FRAME_SKIP, RC_FLAG_RESET,
    VA_PICTURE_H264_SHORT_TERM_REFERENCE,
};
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::pixel_convert::FrameSize;

/// `log2_max_frame_num_minus4`: `frame_num` wraps at 65536.
pub(super) const LOG2_MAX_FRAME_NUM_MINUS4: u32 = 12;
pub(super) const MAX_FRAME_NUM: u32 = 1 << (LOG2_MAX_FRAME_NUM_MINUS4 + 4);
/// The "no periodic intra picture" period given to the driver's rate
/// control; this program sets every picture's type itself (assumption
/// A-VA-GOP: the driver does not insert I pictures on its own; Sunshine
/// gives FFmpeg `gop_size = INT16_MAX` for the same purpose, R01).
pub(super) const INTRA_PERIOD: u32 = 32767;
pub(super) const PIC_INIT_QP: u8 = 26;

/// Macroblock geometry of one encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub size: FrameSize,
    pub width_mbs: u16,
    pub height_mbs: u16,
}

impl Layout {
    pub(super) fn new(size: FrameSize) -> Self {
        Self {
            size,
            width_mbs: size.width.div_ceil(16) as u16,
            height_mbs: size.height.div_ceil(16) as u16,
        }
    }

    /// The surface size: whole macroblocks.
    pub(super) fn coded(&self) -> FrameSize {
        FrameSize::new(
            u32::from(self.width_mbs) * 16,
            u32::from(self.height_mbs) * 16,
        )
    }

    pub(super) fn macroblocks(&self) -> u32 {
        u32::from(self.width_mbs) * u32::from(self.height_mbs)
    }

    /// `frame_crop_right_offset`, `frame_crop_bottom_offset` in 4:2:0
    /// frame units of two samples (7.4.2.1.1, CropUnitX = CropUnitY = 2).
    pub(super) fn crop(&self) -> (u32, u32) {
        let coded = self.coded();
        (
            (coded.width - self.size.width) / 2,
            (coded.height - self.size.height) / 2,
        )
    }
}

/// `profile_idc` and the constraint flags byte (A.2.1, A.2.4).
pub(super) fn profile_idc(profile: H264Profile) -> (u8, u8) {
    match profile {
        // constraint_set0_flag and constraint_set1_flag: Constrained Baseline.
        H264Profile::ConstrainedBaseline => (66, 0b1100_0000),
        H264Profile::High => (100, 0),
    }
}

/// The sequence parameter buffer for `layout` (fields as in `sps`).
pub(super) fn sequence_params(layout: &Layout, level: u8, rate: &RateParams) -> VaEncSequenceH264 {
    let mut seq = VaEncSequenceH264::zeroed();
    seq.level_idc = level;
    seq.intra_period = INTRA_PERIOD;
    seq.intra_idr_period = INTRA_PERIOD;
    seq.ip_period = 1;
    seq.bits_per_second = rate.bits_per_second;
    seq.max_num_ref_frames = 1;
    seq.picture_width_in_mbs = layout.width_mbs;
    seq.picture_height_in_mbs = layout.height_mbs;
    // chroma_format_idc 1, frame_mbs_only_flag, direct_8x8_inference_flag,
    // log2_max_frame_num_minus4, pic_order_cnt_type 2.
    seq.seq_fields = 1 | (1 << 2) | (1 << 5) | (LOG2_MAX_FRAME_NUM_MINUS4 << 6) | (2 << 10);
    let (right, bottom) = layout.crop();
    if right != 0 || bottom != 0 {
        seq.frame_cropping_flag = 1;
        seq.frame_crop_right_offset = right;
        seq.frame_crop_bottom_offset = bottom;
    }
    seq.vui_parameters_present_flag = 1;
    // bitstream_restriction_flag, log2_max_mv_length 16/16,
    // motion_vectors_over_pic_boundaries_flag.
    seq.vui_fields = (1 << 2) | (16 << 3) | (16 << 8) | (1 << 15);
    seq
}

/// The SPS NAL unit (Annex B) matching `sequence_params`.
pub(super) fn sps(layout: &Layout, profile: H264Profile, level: u8) -> Vec<u8> {
    let mut w = BitWriter::default();
    let (idc, constraints) = profile_idc(profile);
    w.bits(8, u32::from(idc));
    w.bits(8, u32::from(constraints));
    w.bits(8, u32::from(level));
    w.ue(0); // seq_parameter_set_id
    if profile == H264Profile::High {
        w.ue(1); // chroma_format_idc
        w.ue(0); // bit_depth_luma_minus8
        w.ue(0); // bit_depth_chroma_minus8
        w.flag(false); // qpprime_y_zero_transform_bypass_flag
        w.flag(false); // seq_scaling_matrix_present_flag
    }
    w.ue(LOG2_MAX_FRAME_NUM_MINUS4);
    w.ue(2); // pic_order_cnt_type
    w.ue(1); // max_num_ref_frames
    w.flag(false); // gaps_in_frame_num_value_allowed_flag
    w.ue(u32::from(layout.width_mbs) - 1);
    w.ue(u32::from(layout.height_mbs) - 1);
    w.flag(true); // frame_mbs_only_flag
    w.flag(true); // direct_8x8_inference_flag
    let (right, bottom) = layout.crop();
    w.flag(right != 0 || bottom != 0);
    if right != 0 || bottom != 0 {
        w.ue(0);
        w.ue(right);
        w.ue(0);
        w.ue(bottom);
    }
    w.flag(true); // vui_parameters_present_flag
    vui(&mut w);
    annex_b_nal(3, NAL_SPS, &w.finish())
}

/// VUI (E.1.1): BT.709 primaries, transfer and matrix in limited range;
/// no timing (the frame rate follows the viewer); no reordering.
fn vui(w: &mut BitWriter) {
    w.flag(false); // aspect_ratio_info_present_flag
    w.flag(false); // overscan_info_present_flag
    w.flag(true); // video_signal_type_present_flag
    w.bits(3, 5); // video_format: unspecified
    w.flag(false); // video_full_range_flag
    w.flag(true); // colour_description_present_flag
    w.bits(8, 1); // colour_primaries: BT.709
    w.bits(8, 1); // transfer_characteristics: BT.709
    w.bits(8, 1); // matrix_coefficients: BT.709
    w.flag(false); // chroma_loc_info_present_flag
    w.flag(false); // timing_info_present_flag
    w.flag(false); // nal_hrd_parameters_present_flag
    w.flag(false); // vcl_hrd_parameters_present_flag
    w.flag(false); // pic_struct_present_flag
    w.flag(true); // bitstream_restriction_flag
    w.flag(true); // motion_vectors_over_pic_boundaries_flag
    w.ue(2); // max_bytes_per_pic_denom (default)
    w.ue(1); // max_bits_per_mb_denom (default)
    w.ue(16); // log2_max_mv_length_horizontal
    w.ue(16); // log2_max_mv_length_vertical
    w.ue(0); // max_num_reorder_frames
    w.ue(1); // max_dec_frame_buffering
}

/// Whether the profile uses CABAC and the 8×8 transform (High) or CAVLC
/// only (Constrained Baseline).
pub(super) fn high_tools(profile: H264Profile) -> bool {
    profile == H264Profile::High
}

/// The PPS NAL unit (Annex B) matching the picture parameters.
pub(super) fn pps(profile: H264Profile) -> Vec<u8> {
    let mut w = BitWriter::default();
    let high = high_tools(profile);
    w.ue(0); // pic_parameter_set_id
    w.ue(0); // seq_parameter_set_id
    w.flag(high); // entropy_coding_mode_flag
    w.flag(false); // bottom_field_pic_order_in_frame_present_flag
    w.ue(0); // num_slice_groups_minus1
    w.ue(0); // num_ref_idx_l0_default_active_minus1
    w.ue(0); // num_ref_idx_l1_default_active_minus1
    w.flag(false); // weighted_pred_flag
    w.bits(2, 0); // weighted_bipred_idc
    w.se(i32::from(PIC_INIT_QP) - 26); // pic_init_qp_minus26
    w.se(0); // pic_init_qs_minus26
    w.se(0); // chroma_qp_index_offset
    w.flag(true); // deblocking_filter_control_present_flag
    w.flag(false); // constrained_intra_pred_flag
    w.flag(false); // redundant_pic_cnt_present_flag
    if high {
        w.flag(true); // transform_8x8_mode_flag
        w.flag(false); // pic_scaling_matrix_present_flag
        w.se(0); // second_chroma_qp_index_offset
    }
    annex_b_nal(3, NAL_PPS, &w.finish())
}

/// The `pic_fields` word of `VAEncPictureParameterBufferH264`.
pub(super) fn picture_fields(profile: H264Profile, idr: bool) -> u32 {
    let high = high_tools(profile);
    u32::from(idr) // idr_pic_flag
        | (1 << 1) // reference_pic_flag
        | (u32::from(high) << 3) // entropy_coding_mode_flag
        | (u32::from(high) << 8) // transform_8x8_mode_flag
        | (1 << 9) // deblocking_filter_control_present_flag
}

/// The state of one picture to encode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PictureState {
    pub idr: bool,
    pub frame_num: u32,
    pub frames_since_idr: u32,
    pub idr_pic_id: u16,
    /// Reconstructed surface of this picture.
    pub recon: VaId,
    /// Reconstructed surface of the previous picture (P pictures).
    pub reference: Option<VaId>,
    pub coded: VaId,
}

impl PictureState {
    fn current(&self) -> VaPictureH264 {
        let poc = (self.frames_since_idr * 2) as i32;
        VaPictureH264 {
            picture_id: self.recon,
            frame_idx: self.frame_num,
            top_field_order_cnt: poc,
            bottom_field_order_cnt: poc,
            ..VaPictureH264::default()
        }
    }

    /// The one short-term reference of a P picture.
    fn reference(&self) -> Option<VaPictureH264> {
        let poc = (self.frames_since_idr * 2) as i32 - 2;
        self.reference
            .filter(|_| !self.idr)
            .map(|surface| VaPictureH264 {
                picture_id: surface,
                frame_idx: (self.frame_num + MAX_FRAME_NUM - 1) % MAX_FRAME_NUM,
                flags: VA_PICTURE_H264_SHORT_TERM_REFERENCE,
                top_field_order_cnt: poc,
                bottom_field_order_cnt: poc,
                ..VaPictureH264::default()
            })
    }
}

/// The picture parameter buffer of `state`.
pub(super) fn picture_params(state: &PictureState, profile: H264Profile) -> VaEncPictureH264 {
    let mut picture = VaEncPictureH264::zeroed();
    picture.curr_pic = state.current();
    picture.reference_frames = [VaPictureH264::invalid(); 16];
    if let Some(reference) = state.reference() {
        picture.reference_frames[0] = reference;
    }
    picture.coded_buf = state.coded;
    picture.frame_num = state.frame_num as u16;
    picture.pic_init_qp = PIC_INIT_QP;
    picture.pic_fields = picture_fields(profile, state.idr);
    picture
}

/// The one slice of the picture: every macroblock, I for an IDR, else P
/// with the previous picture as the only reference.
pub(super) fn slice_params(state: &PictureState, layout: &Layout) -> VaEncSliceH264 {
    let mut slice = VaEncSliceH264::zeroed();
    slice.num_macroblocks = layout.macroblocks();
    slice.macroblock_info = VA_INVALID_ID;
    slice.slice_type = if state.idr { 2 } else { 0 };
    slice.idr_pic_id = state.idr_pic_id;
    slice.ref_pic_list0 = [VaPictureH264::invalid(); 32];
    slice.ref_pic_list1 = [VaPictureH264::invalid(); 32];
    if let Some(reference) = state.reference() {
        slice.ref_pic_list0[0] = reference;
    }
    slice
}

/// The three misc parameters for `rate` (B-21): CBR at the target, a
/// buffer of one frame, frame skipping off; `reset` on a change at run time.
pub(super) fn misc_params(
    rate: &RateParams,
    reset: bool,
) -> (VaEncMiscRateControl, VaEncMiscHrd, VaEncMiscFrameRate) {
    let window_ms = u64::from(rate.buffer_bits) * 1000 / u64::from(rate.bits_per_second.max(1));
    let control = VaEncMiscRateControl {
        bits_per_second: rate.bits_per_second,
        target_percentage: 100,
        window_size: window_ms.max(1) as u32,
        rc_flags: RC_FLAG_DISABLE_FRAME_SKIP | if reset { RC_FLAG_RESET } else { 0 },
        ..Default::default()
    };
    let hrd = VaEncMiscHrd {
        initial_buffer_fullness: rate.initial_fullness,
        buffer_size: rate.buffer_bits,
        ..Default::default()
    };
    let frame_rate = VaEncMiscFrameRate {
        framerate: rate.fps.min(0xffff) | (1 << 16),
        ..Default::default()
    };
    (control, hrd, frame_rate)
}
