//! The VA-API decode parameter buffers of one HEVC picture (plan
//! 2026-11-14 „Bildkette“ BK4b, B-38): `VAPictureParameterBufferHEVC` and
//! `VASliceParameterBufferHEVC` (libva 2.20.0 `va_dec_hevc.h`) from the
//! parsed SPS, PPS, slice segment headers and reference pictures, filled as
//! FFmpeg's `libavcodec/vaapi_hevc.c` fills them as known to the author
//! (assumption A-VA-HEVC-DEC: `IntraPicFlag` = IRAP, `NoBiPredFlag` 0, the
//! slice data buffer holds the whole NAL unit with its emulation prevention
//! bytes, `slice_data_byte_offset` counts the header without them, one
//! parameter/data pair per slice segment in one `vaRenderPicture`).
use std::io;

use super::vaapi_hevc_abi::{VaPictureHevc, VaPictureParameterHevc, VaSliceParameterHevc, Zeroed};
use super::vaapi_hevc_dpb::{ref_lists, PictureRefs};
use super::vaapi_hevc_slice::SliceHeader;
use super::vaapi_hevc_syntax::{Pps, Sps};
use crate::screen::core::wire::invalid;

/// Tile column widths and row heights in CTBs, minus one each (6.5.1:
/// uniform spacing or the PPS's explicit sizes with the rest last).
pub(super) fn tile_sizes(sps: &Sps, pps: &Pps) -> (Vec<u32>, Vec<u32>) {
    let Some((widths, heights, uniform, _)) = &pps.tiles else {
        return (Vec::new(), Vec::new());
    };
    let (columns, rows) = sps.ctbs();
    let split = |explicit: &[u32], total: u32| -> Vec<u32> {
        let parts = explicit.len() as u32 + 1;
        // Saturating: a PPS naming more tiles than CTBs (a bitstream
        // violation) yields no panic; the driver then refuses the picture.
        if *uniform {
            (0..parts)
                .map(|i| {
                    (((i + 1) * total) / parts)
                        .saturating_sub((i * total) / parts)
                        .saturating_sub(1)
                })
                .collect()
        } else {
            let used = explicit.iter().fold(0u32, |sum, value| {
                sum.saturating_add(value.saturating_add(1))
            });
            let mut out = explicit.to_vec();
            out.push(total.saturating_sub(used).saturating_sub(1));
            out
        }
    };
    (split(widths, columns), split(heights, rows))
}

/// The picture parameters of the picture decoded into `target`.
pub(super) fn picture_params(
    sps: &Sps,
    pps: &Pps,
    first: &SliceHeader,
    refs: &PictureRefs,
    target: u32,
) -> io::Result<VaPictureParameterHevc> {
    let mut p = VaPictureParameterHevc::zeroed();
    p.curr_pic = VaPictureHevc {
        picture_id: target,
        pic_order_cnt: refs.poc,
        ..VaPictureHevc::default()
    };
    p.reference_frames = [VaPictureHevc::invalid(); 15];
    for (slot, frame) in p.reference_frames.iter_mut().zip(&refs.frames) {
        *slot = *frame;
    }
    p.pic_width_in_luma_samples =
        u16::try_from(sps.width).map_err(|_| invalid("HEVC picture too wide"))?;
    p.pic_height_in_luma_samples =
        u16::try_from(sps.height).map_err(|_| invalid("HEVC picture too high"))?;
    let tiles = pps.tiles.as_ref();
    let pcm = sps.pcm;
    let flag = |on: bool, bit: u32| u32::from(on) << bit;
    p.pic_fields = 1 // chroma_format_idc
        | flag(pcm.is_some(), 3)
        | flag(pps.transform_skip, 5)
        | flag(sps.amp, 6)
        | flag(sps.strong_intra_smoothing, 7)
        | flag(pps.sign_data_hiding, 8)
        | flag(pps.constrained_intra_pred, 9)
        | flag(pps.cu_qp_delta.is_some(), 10)
        | flag(pps.transquant_bypass, 13)
        | flag(tiles.is_some(), 14)
        | flag(pps.entropy_coding_sync, 15)
        | flag(pps.loop_filter_across_slices, 16)
        | flag(tiles.is_some_and(|t| t.3), 17)
        | flag(pcm.is_some_and(|pcm| pcm.4), 18)
        | flag(sps.num_reorder_pics == 0, 19);
    p.sps_max_dec_pic_buffering_minus1 = (sps.max_dec_pic_buffering.max(1) - 1) as u8;
    if let Some((luma, chroma, min, diff, _)) = pcm {
        p.pcm_sample_bit_depth_luma_minus1 = luma as u8;
        p.pcm_sample_bit_depth_chroma_minus1 = chroma as u8;
        p.log2_min_pcm_luma_coding_block_size_minus3 = min as u8;
        p.log2_diff_max_min_pcm_luma_coding_block_size = diff as u8;
    }
    p.log2_min_luma_coding_block_size_minus3 = (sps.log2_min_cb - 3) as u8;
    p.log2_diff_max_min_luma_coding_block_size = sps.log2_diff_max_min_cb as u8;
    p.log2_min_transform_block_size_minus2 = (sps.log2_min_tb - 2) as u8;
    p.log2_diff_max_min_transform_block_size = sps.log2_diff_max_min_tb as u8;
    p.max_transform_hierarchy_depth_intra = sps.depth_intra as u8;
    p.max_transform_hierarchy_depth_inter = sps.depth_inter as u8;
    p.init_qp_minus26 = pps.init_qp_minus26 as i8;
    p.diff_cu_qp_delta_depth = pps.cu_qp_delta.unwrap_or(0) as u8;
    p.pps_cb_qp_offset = pps.cb_qp_offset as i8;
    p.pps_cr_qp_offset = pps.cr_qp_offset as i8;
    p.log2_parallel_merge_level_minus2 = (pps.log2_parallel_merge_level - 2) as u8;
    let (columns, rows) = tile_sizes(sps, pps);
    p.num_tile_columns_minus1 = columns.len().saturating_sub(1) as u8;
    p.num_tile_rows_minus1 = rows.len().saturating_sub(1) as u8;
    for (slot, value) in p.column_width_minus1.iter_mut().zip(&columns) {
        *slot = *value as u16;
    }
    for (slot, value) in p.row_height_minus1.iter_mut().zip(&rows) {
        *slot = *value as u16;
    }
    let irap = first.irap();
    p.slice_parsing_fields = flag(pps.lists_modification_present, 0)
        | flag(sps.long_term_present, 1)
        | flag(sps.temporal_mvp, 2)
        | flag(pps.cabac_init_present, 3)
        | flag(pps.output_flag_present, 4)
        | flag(pps.dependent_slices, 5)
        | flag(pps.slice_chroma_qp_offsets_present, 6)
        | flag(sps.sao, 7)
        | flag(pps.deblocking_override_enabled, 8)
        | flag(pps.deblocking_disabled, 9)
        | flag(pps.slice_header_extension_present, 10)
        | flag(irap, 11)
        | flag(first.idr(), 12)
        | flag(irap, 13);
    p.log2_max_pic_order_cnt_lsb_minus4 = (sps.log2_max_poc_lsb - 4) as u8;
    p.num_short_term_ref_pic_sets = sps.st_rps.len() as u8;
    p.num_long_term_ref_pic_sps = sps.num_long_term_sps as u8;
    p.num_ref_idx_l0_default_active_minus1 = (pps.num_ref_idx_l0_default - 1) as u8;
    p.num_ref_idx_l1_default_active_minus1 = (pps.num_ref_idx_l1_default - 1) as u8;
    p.pps_beta_offset_div2 = pps.beta_offset_div2 as i8;
    p.pps_tc_offset_div2 = pps.tc_offset_div2 as i8;
    p.num_extra_slice_header_bits = pps.extra_slice_header_bits as u8;
    p.st_rps_bits = first.st_rps_bits;
    Ok(p)
}

/// Emulation prevention bytes of `unit` (escaped, header included) before
/// the first `rbsp_bytes` bytes of its RBSP after the two header bytes.
pub(super) fn emulation_bytes_before(unit: &[u8], rbsp_bytes: usize) -> usize {
    let (mut kept, mut removed, mut zeros) = (0, 0, 0);
    for &byte in unit.iter().skip(2) {
        if kept >= rbsp_bytes {
            break;
        }
        if zeros >= 2 && byte == 3 {
            removed += 1;
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        kept += 1;
    }
    removed
}

/// The slice parameters of one slice segment NAL unit `unit`.
pub(super) fn slice_params(
    h: &SliceHeader,
    refs: &PictureRefs,
    unit: &[u8],
    last: bool,
) -> io::Result<VaSliceParameterHevc> {
    let mut s = VaSliceParameterHevc::zeroed();
    s.slice_data_size = u32::try_from(unit.len()).map_err(|_| invalid("HEVC slice too large"))?;
    s.slice_data_byte_offset = h.header_bytes as u32;
    s.slice_segment_address = h.address;
    s.ref_pic_list = ref_lists(h, refs)?;
    let flag = |on: bool, bit: u32| u32::from(on) << bit;
    s.long_slice_flags = flag(last, 0)
        | flag(h.dependent, 1)
        | (u32::from(h.slice_type) << 2)
        | flag(h.sao_luma, 6)
        | flag(h.sao_chroma, 7)
        | flag(h.mvd_l1_zero, 8)
        | flag(h.cabac_init, 9)
        | flag(h.temporal_mvp, 10)
        | flag(h.deblocking_disabled, 11)
        | flag(h.collocated_from_l0, 12)
        | flag(h.loop_filter_across_slices, 13);
    s.collocated_ref_idx = if h.temporal_mvp {
        h.collocated_ref_idx as u8
    } else {
        0xff
    };
    s.num_ref_idx_l0_active_minus1 = h.num_ref_idx[0].saturating_sub(1) as u8;
    s.num_ref_idx_l1_active_minus1 = h.num_ref_idx[1].saturating_sub(1) as u8;
    s.slice_qp_delta = h.qp_delta as i8;
    s.slice_cb_qp_offset = h.cb_qp_offset as i8;
    s.slice_cr_qp_offset = h.cr_qp_offset as i8;
    s.slice_beta_offset_div2 = h.beta_offset_div2 as i8;
    s.slice_tc_offset_div2 = h.tc_offset_div2 as i8;
    s.five_minus_max_num_merge_cand = h.five_minus_max_merge_cand as u8;
    s.num_entry_point_offsets =
        u16::try_from(h.entry_points).map_err(|_| invalid("HEVC entry points out of range"))?;
    s.slice_data_num_emu_prevn_bytes = emulation_bytes_before(unit, h.header_bytes - 2) as u16;
    Ok(s)
}
