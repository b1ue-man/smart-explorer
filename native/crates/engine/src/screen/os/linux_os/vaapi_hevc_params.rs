//! The libva parameter buffers of the VA-API HEVC encoder (plan
//! 2026-11-14 „Bildkette“ BK4b): `VAEncSequenceParameterBufferHEVC`,
//! `VAEncPictureParameterBufferHEVC` and `VAEncSliceParameterBufferHEVC`
//! (libva 2.20.0 `va_enc_hevc.h`) with the same values as the VPS, SPS,
//! PPS and slice header of `vaapi_hevc_headers.rs`, so a driver that
//! writes its own headers writes the same ones; and the state of one
//! picture (IDR or P, POC, reconstructed and reference surface).
use super::hw_encode_common::RateParams;
use super::vaapi_ffi::VaId;
use super::vaapi_h264_headers::INTRA_PERIOD;
use super::vaapi_hevc_abi::{
    VaEncPictureHevc, VaEncSequenceHevc, VaEncSliceHevc, VaPictureHevc, Zeroed,
};
use super::vaapi_hevc_headers::{
    HevcLayout, HevcTools, MAX_MERGE_CAND, NAL_IDR_W_RADL, NAL_TRAIL_R, PIC_INIT_QP, SLICE_B,
    SLICE_I, SLICE_P,
};

/// The state of one picture to encode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HevcPicture {
    pub idr: bool,
    /// Picture order count: pictures since the IDR.
    pub poc: u32,
    pub recon: VaId,
    /// Reconstructed surface of the previous picture (inter pictures).
    pub reference: Option<VaId>,
    pub coded: VaId,
}

impl HevcPicture {
    fn current(&self) -> VaPictureHevc {
        VaPictureHevc {
            picture_id: self.recon,
            pic_order_cnt: self.poc as i32,
            ..VaPictureHevc::default()
        }
    }

    fn reference(&self) -> Option<VaPictureHevc> {
        self.reference
            .filter(|_| !self.idr)
            .map(|surface| VaPictureHevc {
                picture_id: surface,
                pic_order_cnt: self.poc as i32 - 1,
                ..VaPictureHevc::default()
            })
    }

    /// `slice_type` of the picture's one slice.
    pub(super) fn slice_type(&self, tools: &HevcTools) -> u8 {
        match (self.idr, tools.gpb) {
            (true, _) => SLICE_I,
            (false, true) => SLICE_B,
            (false, false) => SLICE_P,
        }
    }

    pub(super) fn nal_type(&self) -> u8 {
        if self.idr {
            NAL_IDR_W_RADL
        } else {
            NAL_TRAIL_R
        }
    }
}

/// The sequence parameter buffer (fields as `sps`).
pub(super) fn sequence_params(
    layout: &HevcLayout,
    tools: &HevcTools,
    level_idc: u8,
    rate: &RateParams,
) -> VaEncSequenceHevc {
    let mut seq = VaEncSequenceHevc::zeroed();
    seq.general_profile_idc = 1;
    seq.general_level_idc = level_idc;
    seq.intra_period = INTRA_PERIOD;
    seq.intra_idr_period = INTRA_PERIOD;
    seq.ip_period = 1;
    seq.bits_per_second = rate.bits_per_second;
    let coded = layout.coded();
    seq.pic_width_in_luma_samples = coded.width as u16;
    seq.pic_height_in_luma_samples = coded.height as u16;
    seq.seq_fields = 1 // chroma_format_idc
        | (u32::from(tools.strong_intra_smoothing) << 10)
        | (u32::from(tools.amp) << 11)
        | (u32::from(tools.sao) << 12)
        | (u32::from(tools.temporal_mvp) << 15)
        | (1 << 16); // low_delay_seq
    seq.log2_min_luma_coding_block_size_minus3 = (layout.min_cb_log2 - 3) as u8;
    seq.log2_diff_max_min_luma_coding_block_size = (layout.ctb_log2 - layout.min_cb_log2) as u8;
    seq.log2_min_transform_block_size_minus2 = (layout.min_tb_log2 - 2) as u8;
    seq.log2_diff_max_min_transform_block_size = (layout.max_tb_log2 - layout.min_tb_log2) as u8;
    seq.max_transform_hierarchy_depth_inter = layout.depth_inter as u8;
    seq.max_transform_hierarchy_depth_intra = layout.depth_intra as u8;
    seq.vui_parameters_present_flag = 1;
    // bitstream_restriction_flag, motion_vectors_over_pic_boundaries_flag,
    // restricted_ref_pic_lists_flag, log2_max_mv_length 15/15.
    seq.vui_fields = (1 << 4) | (1 << 6) | (1 << 7) | (15 << 8) | (15 << 13);
    seq.max_bytes_per_pic_denom = 2;
    seq.max_bits_per_min_cu_denom = 1;
    seq
}

/// The picture parameter buffer of `picture` (fields as `pps`).
pub(super) fn picture_params(tools: &HevcTools, picture: &HevcPicture) -> VaEncPictureHevc {
    let mut out = VaEncPictureHevc::zeroed();
    out.decoded_curr_pic = picture.current();
    out.reference_frames = [VaPictureHevc::invalid(); 15];
    let reference = picture.reference();
    if let Some(mut reference) = reference {
        reference.flags = super::vaapi_hevc_abi::VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE;
        out.reference_frames[0] = reference;
    }
    out.coded_buf = picture.coded;
    out.collocated_ref_pic_index = if reference.is_some() && tools.temporal_mvp {
        0
    } else {
        0xff
    };
    out.pic_init_qp = PIC_INIT_QP;
    out.nal_unit_type = picture.nal_type();
    let coding_type = if picture.idr { 1 } else { 2 };
    out.pic_fields = u32::from(picture.idr)
        | (coding_type << 1)
        | (1 << 4) // reference_pic_flag
        | (u32::from(tools.sign_data_hiding) << 6)
        | (u32::from(tools.constrained_intra_pred) << 7)
        | (u32::from(tools.transform_skip) << 8)
        | (u32::from(tools.cu_qp_delta) << 9)
        | (1 << 18); // screen_content_flag: the content is a screen
    out
}

/// The one slice of `picture`: every CTU, the previous picture in list 0
/// (and in list 1 for a low-delay B slice).
pub(super) fn slice_params(
    layout: &HevcLayout,
    tools: &HevcTools,
    picture: &HevcPicture,
) -> VaEncSliceHevc {
    let mut slice = VaEncSliceHevc::zeroed();
    slice.num_ctu_in_slice = layout.ctus();
    let slice_type = picture.slice_type(tools);
    slice.slice_type = slice_type;
    slice.ref_pic_list0 = [VaPictureHevc::invalid(); 15];
    slice.ref_pic_list1 = [VaPictureHevc::invalid(); 15];
    if let Some(reference) = picture.reference() {
        slice.ref_pic_list0[0] = reference;
        if slice_type == SLICE_B {
            slice.ref_pic_list1[0] = reference;
        }
    }
    slice.max_num_merge_cand = MAX_MERGE_CAND;
    let temporal_mvp = tools.temporal_mvp && !picture.idr;
    slice.slice_fields = 1 // last_slice_of_pic_flag
        | (u32::from(temporal_mvp) << 4)
        | (u32::from(tools.sao) << 5)
        | (u32::from(tools.sao) << 6)
        | (1 << 13); // collocated_from_l0_flag
    slice
}
