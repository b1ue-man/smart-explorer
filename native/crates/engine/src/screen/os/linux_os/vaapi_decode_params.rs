//! The VA-API decode parameter buffers of one H.264 picture (BK4, B-38):
//! `VAPictureParameterBufferH264`, `VAIQMatrixBufferH264` and
//! `VASliceParameterBufferH264` (libva 2.20.0 `va.h`) from the parsed
//! SPS, PPS, slice header and reference pictures, filled as FFmpeg's
//! `libavcodec/vaapi_h264.c` fills them (assumption A-VA-DEC in the BK4
//! report: `slice_data_bit_offset` counts the NAL header byte and the
//! slice header without emulation prevention bytes, the slice data buffer
//! holds the whole NAL unit without start code; the drivers skip the
//! emulation prevention bytes themselves).
use std::io;

use super::vaapi_h264_abi::{
    VaIqMatrixH264, VaPictureH264, VaPictureParameterH264, VaSliceParameterH264, Zeroed,
    VA_PICTURE_H264_LONG_TERM_REFERENCE, VA_PICTURE_H264_SHORT_TERM_REFERENCE,
    VA_SLICE_DATA_FLAG_ALL,
};
use super::vaapi_h264_dpb::{Dpb, RefPicture};
use super::vaapi_h264_slice::SliceHeader;
use super::vaapi_h264_syntax::{effective_lists, Pps, Sps};
use crate::screen::core::wire::invalid;

/// A reference picture as VA-API names it.
pub(super) fn reference(picture: &RefPicture) -> VaPictureH264 {
    let (flags, frame_idx) = match picture.long_term {
        Some(idx) => (VA_PICTURE_H264_LONG_TERM_REFERENCE, idx),
        None => (VA_PICTURE_H264_SHORT_TERM_REFERENCE, picture.frame_num),
    };
    VaPictureH264 {
        picture_id: picture.surface,
        frame_idx,
        flags,
        top_field_order_cnt: picture.top_poc,
        bottom_field_order_cnt: picture.bottom_poc,
        ..VaPictureH264::default()
    }
}

/// The `seq_fields` word of the decode picture parameters.
pub(super) fn sequence_fields(sps: &Sps) -> u32 {
    let poc_lsb = sps.log2_max_poc_lsb.saturating_sub(4);
    (sps.chroma_format_idc & 3)
        | (u32::from(sps.gaps_allowed) << 3)
        | (1 << 4) // frame_mbs_only_flag
        | (u32::from(sps.direct_8x8_inference) << 6)
        | ((sps.log2_max_frame_num - 4) << 8)
        | ((sps.poc_type & 3) << 12)
        | ((poc_lsb & 0xf) << 14)
        | (u32::from(sps.delta_pic_order_always_zero) << 18)
}

/// The `pic_fields` word of the decode picture parameters.
pub(super) fn picture_fields(pps: &Pps, reference: bool) -> u32 {
    u32::from(pps.entropy_coding_mode)
        | (u32::from(pps.weighted_pred) << 1)
        | ((pps.weighted_bipred_idc & 3) << 2)
        | (u32::from(pps.transform_8x8_mode) << 4)
        | (u32::from(pps.constrained_intra_pred) << 6)
        | (u32::from(pps.bottom_field_pic_order_in_frame_present) << 7)
        | (u32::from(pps.deblocking_filter_control_present) << 8)
        | (u32::from(pps.redundant_pic_cnt_present) << 9)
        | (u32::from(reference) << 10)
}

/// The picture parameters of the picture decoded into `target`.
pub(super) fn picture_params(
    sps: &Sps,
    pps: &Pps,
    slice: &SliceHeader,
    dpb: &Dpb,
    target: u32,
    poc: (i32, i32),
) -> VaPictureParameterH264 {
    let mut picture = VaPictureParameterH264::zeroed();
    picture.curr_pic = VaPictureH264 {
        picture_id: target,
        frame_idx: slice.frame_num,
        top_field_order_cnt: poc.0,
        bottom_field_order_cnt: poc.1,
        ..VaPictureH264::default()
    };
    picture.reference_frames = [VaPictureH264::invalid(); 16];
    for (slot, entry) in picture.reference_frames.iter_mut().zip(&dpb.refs) {
        *slot = reference(entry);
    }
    picture.picture_width_in_mbs_minus1 = (sps.width_mbs - 1) as u16;
    picture.picture_height_in_mbs_minus1 = (sps.height_mbs - 1) as u16;
    picture.num_ref_frames = sps.max_num_ref_frames as u8;
    picture.seq_fields = sequence_fields(sps);
    picture.pic_init_qp_minus26 = pps.pic_init_qp_minus26 as i8;
    picture.pic_init_qs_minus26 = pps.pic_init_qs_minus26 as i8;
    picture.chroma_qp_index_offset = pps.chroma_qp_index_offset as i8;
    picture.second_chroma_qp_index_offset = pps.second_chroma_qp_index_offset as i8;
    picture.pic_fields = picture_fields(pps, slice.nal_ref_idc != 0);
    picture.frame_num = slice.frame_num as u16;
    picture
}

/// The scaling lists of the picture (flat without a matrix in the stream).
pub(super) fn iq_matrix(sps: &Sps, pps: &Pps) -> VaIqMatrixH264 {
    let lists = effective_lists(sps, pps);
    let mut matrix = VaIqMatrixH264::zeroed();
    matrix.scaling_list4x4 = lists.list4x4;
    matrix.scaling_list8x8 = lists.list8x8;
    matrix
}

/// The slice parameters of one slice NAL unit of `length` bytes with its
/// list 0 (empty for an I slice).
pub(super) fn slice_params(
    slice: &SliceHeader,
    list0: &[RefPicture],
    length: usize,
) -> io::Result<VaSliceParameterH264> {
    let mut parameters = VaSliceParameterH264::zeroed();
    parameters.slice_data_size =
        u32::try_from(length).map_err(|_| invalid("H.264 slice too large"))?;
    parameters.slice_data_offset = 0;
    parameters.slice_data_flag = VA_SLICE_DATA_FLAG_ALL;
    parameters.slice_data_bit_offset =
        u16::try_from(slice.header_bits).map_err(|_| invalid("H.264 slice header too long"))?;
    parameters.first_mb_in_slice =
        u16::try_from(slice.first_mb).map_err(|_| invalid("H.264 first_mb_in_slice too large"))?;
    parameters.slice_type = slice.slice_type as u8;
    parameters.num_ref_idx_l0_active_minus1 = if slice.slice_type == 0 {
        slice.num_ref_idx_l0_minus1 as u8
    } else {
        0
    };
    parameters.cabac_init_idc = slice.cabac_init_idc as u8;
    parameters.slice_qp_delta = slice.slice_qp_delta as i8;
    parameters.disable_deblocking_filter_idc = slice.disable_deblocking_filter_idc as u8;
    parameters.slice_alpha_c0_offset_div2 = slice.alpha_offset_div2 as i8;
    parameters.slice_beta_offset_div2 = slice.beta_offset_div2 as i8;
    parameters.ref_pic_list0 = [VaPictureH264::invalid(); 32];
    parameters.ref_pic_list1 = [VaPictureH264::invalid(); 32];
    for (slot, entry) in parameters.ref_pic_list0.iter_mut().zip(list0) {
        *slot = reference(entry);
    }
    if let Some(weights) = &slice.weights {
        parameters.luma_log2_weight_denom = weights.luma_denom as u8;
        parameters.chroma_log2_weight_denom = weights.chroma_denom as u8;
        let luma_default = 1i32 << weights.luma_denom;
        let chroma_default = 1i32 << weights.chroma_denom;
        parameters.luma_weight_l0_flag = u8::from(weights.luma.iter().any(Option::is_some));
        parameters.chroma_weight_l0_flag = u8::from(weights.chroma.iter().any(Option::is_some));
        for (index, entry) in weights.luma.iter().enumerate().take(32) {
            let (weight, offset) = entry.unwrap_or((luma_default, 0));
            parameters.luma_weight_l0[index] = weight as i16;
            parameters.luma_offset_l0[index] = offset as i16;
        }
        for (index, entry) in weights.chroma.iter().enumerate().take(32) {
            let pairs = entry.unwrap_or([(chroma_default, 0); 2]);
            for (plane, (weight, offset)) in pairs.iter().enumerate() {
                parameters.chroma_weight_l0[index][plane] = *weight as i16;
                parameters.chroma_offset_l0[index][plane] = *offset as i16;
            }
        }
    }
    Ok(parameters)
}
