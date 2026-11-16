//! VA-API AV1 decoder of the Linux viewer (plan 2026-11-14 „Bildkette“
//! BK4b, B-38): one temporal unit per record (low-overhead format, BK6);
//! the program parses the sequence header, the frame header and the tile
//! group headers (`vaapi_av1_syntax.rs`, `vaapi_av1_frame.rs`) and keeps
//! the eight reference slots (AV1 v1.0.0 with Errata 1, 7.20 and 7.21);
//! the GPU decodes the tiles into NV12, which `vaapi_decode_core.rs` reads
//! back into the plane path. Parameters: `VADecPictureParameterBufferAV1`
//! and one `VASliceParameterBufferAV1` per tile with the tile group's data
//! (offsets relative to the data after the tile group header), as FFmpeg's
//! `libavcodec/vaapi_av1.c` passes them as known to the author (assumption
//! A-VA-AV1-DEC).
use std::io;

use super::h264_bits::BitReader;
use super::vaapi_av1_abi::{VaDecPictureAv1, VaSliceParameterAv1, VaWarpedMotionAv1, Zeroed};
use super::vaapi_av1_bits::{
    le, split_obus, OBU_FRAME, OBU_FRAME_HEADER, OBU_SEQUENCE_HEADER, OBU_TILE_GROUP, OBU_TILE_LIST,
};
use super::vaapi_av1_frame::{parse_frame_header, FrameHeader};
use super::vaapi_av1_syntax::{
    parse_sequence_header, unsupported, RefSlot, SequenceHeader, NUM_REF_FRAMES,
};
use super::vaapi_decode_core::{UnitDecoder, VaDecodeSession};
use super::vaapi_ffi::{
    VaId, VA_INVALID_ID, VA_PICTURE_PARAMETER_BUFFER, VA_SLICE_DATA_BUFFER,
    VA_SLICE_PARAMETER_BUFFER,
};
use super::vaapi_vpp::Buffers;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;

/// Surfaces besides the eight slots: the current frame and one spare.
const EXTRA_SURFACES: usize = 2;

/// The tiles of one tile group: their parameters and the data they index.
#[derive(Clone, Debug)]
pub(super) struct TileGroup<'a> {
    pub start: usize,
    pub end: usize,
    pub tiles: Vec<VaSliceParameterAv1>,
    pub data: &'a [u8],
}

/// `tile_group_obu()` (5.11.1) of `data` for the frame `h`.
pub(super) fn parse_tile_group<'a>(data: &'a [u8], h: &FrameHeader) -> io::Result<TileGroup<'a>> {
    let info = &h.tiles;
    let count = info.count();
    let columns = info.widths.len();
    if count == 0 {
        return Err(invalid("AV1 frame without tiles"));
    }
    let mut r = BitReader::new(data);
    let (start, end) = if count > 1 && r.flag()? {
        let bits = info.cols_log2 + info.rows_log2;
        (r.bits(bits)? as usize, r.bits(bits)? as usize)
    } else {
        (0, count - 1)
    };
    if start > end || end >= count {
        return Err(invalid("AV1 tile group beyond the tiles"));
    }
    let data = data
        .get(r.position().div_ceil(8)..)
        .ok_or_else(|| invalid("truncated AV1 tile group"))?;
    let mut tiles = Vec::with_capacity(end - start + 1);
    let mut at = 0usize;
    for tile in start..=end {
        let size = if tile == end {
            data.len()
                .checked_sub(at)
                .ok_or_else(|| invalid("truncated AV1 tile"))?
        } else {
            let size = usize::try_from(le(data, at, info.size_bytes)?)
                .ok()
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| invalid("AV1 tile size overflow"))?;
            at = at
                .checked_add(info.size_bytes)
                .ok_or_else(|| invalid("AV1 tile offset overflow"))?;
            size
        };
        if size == 0 || at.checked_add(size).is_none_or(|stop| stop > data.len()) {
            return Err(invalid("AV1 tile beyond its tile group"));
        }
        tiles.push(VaSliceParameterAv1 {
            slice_data_size: u32::try_from(size).map_err(|_| invalid("AV1 tile size overflow"))?,
            slice_data_offset: u32::try_from(at)
                .map_err(|_| invalid("AV1 tile offset overflow"))?,
            tile_row: (tile / columns) as u16,
            tile_column: (tile % columns) as u16,
            tg_start: start as u16,
            tg_end: end as u16,
            ..VaSliceParameterAv1::default()
        });
        at += size;
    }
    Ok(TileGroup {
        start,
        end,
        tiles,
        data,
    })
}

/// The picture parameters of the frame `h` decoded into `target`.
pub(super) fn picture_params(
    seq: &SequenceHeader,
    h: &FrameHeader,
    slots: &[RefSlot; NUM_REF_FRAMES],
    target: VaId,
) -> VaDecPictureAv1 {
    let mut p = VaDecPictureAv1::zeroed();
    let flag = |on: bool, bit: u32| u32::from(on) << bit;
    p.order_hint_bits_minus_1 = seq.order_hint_bits.saturating_sub(1) as u8;
    p.matrix_coefficients = seq.matrix_coefficients;
    p.seq_info_fields = flag(seq.use_128x128_superblock, 1)
        | flag(seq.enable_filter_intra, 2)
        | flag(seq.enable_intra_edge_filter, 3)
        | flag(seq.enable_interintra_compound, 4)
        | flag(seq.enable_masked_compound, 5)
        | flag(seq.enable_dual_filter, 6)
        | flag(seq.enable_order_hint, 7)
        | flag(seq.enable_jnt_comp, 8)
        | flag(seq.enable_cdef, 9)
        | flag(seq.color_range, 11)
        | (1 << 12) // subsampling_x
        | (1 << 13) // subsampling_y
        | (u32::from(seq.chroma_sample_position & 1) << 14);
    p.current_frame = target;
    p.current_display_picture = target;
    p.frame_width_minus1 = (h.frame_width - 1) as u16;
    p.frame_height_minus1 = (h.frame_height - 1) as u16;
    for (slot, reference) in p.ref_frame_map.iter_mut().zip(slots) {
        *slot = if reference.valid && !h.intra() {
            reference.surface
        } else {
            VA_INVALID_ID
        };
    }
    p.ref_frame_idx = h.ref_frame_idx;
    p.primary_ref_frame = h.primary_ref_frame;
    p.order_hint = h.order_hint as u8;
    let seg = &h.segmentation;
    p.seg_info.segment_info_fields = flag(seg.enabled, 0)
        | flag(seg.update_map, 1)
        | flag(seg.temporal_update, 2)
        | flag(seg.update_data, 3);
    p.seg_info.feature_data = seg.feature_data;
    for (mask, enabled) in p.seg_info.feature_mask.iter_mut().zip(&seg.feature_enabled) {
        *mask = enabled
            .iter()
            .enumerate()
            .fold(0, |bits, (index, on)| bits | (u8::from(*on) << index));
    }
    p.tile_cols = h.tiles.widths.len() as u8;
    p.tile_rows = h.tiles.heights.len() as u8;
    for (slot, width) in p.width_in_sbs_minus_1.iter_mut().zip(&h.tiles.widths) {
        *slot = (*width - 1) as u16;
    }
    for (slot, height) in p.height_in_sbs_minus_1.iter_mut().zip(&h.tiles.heights) {
        *slot = (*height - 1) as u16;
    }
    p.context_update_tile_id = h.tiles.context_update_tile_id as u16;
    p.pic_info_fields = u32::from(h.frame_type)
        | flag(h.show_frame, 2)
        | flag(h.showable_frame, 3)
        | flag(h.error_resilient, 4)
        | flag(h.disable_cdf_update, 5)
        | flag(h.allow_screen_content_tools, 6)
        | flag(h.force_integer_mv, 7)
        | flag(h.allow_intrabc, 8)
        | flag(h.allow_high_precision_mv, 10)
        | flag(h.is_motion_mode_switchable, 11)
        | flag(h.use_ref_frame_mvs, 12)
        | flag(h.disable_frame_end_update_cdf, 13)
        | flag(h.tiles.uniform, 14)
        | flag(h.allow_warped_motion, 15);
    p.superres_scale_denominator = 8;
    p.interp_filter = h.interpolation_filter;
    p.filter_level = [h.loop_filter_level[0], h.loop_filter_level[1]];
    p.filter_level_u = h.loop_filter_level[2];
    p.filter_level_v = h.loop_filter_level[3];
    p.loop_filter_info_fields =
        h.sharpness | (u8::from(h.delta_enabled) << 3) | (u8::from(h.delta_update) << 4);
    p.ref_deltas = h.ref_deltas;
    p.mode_deltas = h.mode_deltas;
    p.base_qindex = h.base_q_idx;
    [
        p.y_dc_delta_q,
        p.u_dc_delta_q,
        p.u_ac_delta_q,
        p.v_dc_delta_q,
        p.v_ac_delta_q,
    ] = h.delta_q;
    let (using, qm_y, qm_u, qm_v) = h.qmatrix;
    p.qmatrix_fields =
        u16::from(using) | (u16::from(qm_y) << 1) | (u16::from(qm_u) << 5) | (u16::from(qm_v) << 9);
    p.mode_control_fields = flag(h.delta_q_present, 0)
        | (u32::from(h.delta_q_res) << 1)
        | flag(h.delta_lf_present, 3)
        | (u32::from(h.delta_lf_res) << 4)
        | flag(h.delta_lf_multi, 6)
        | (u32::from(h.tx_mode) << 7)
        | flag(h.reference_select, 9)
        | flag(h.reduced_tx_set, 10)
        | flag(h.skip_mode_present, 11);
    p.cdef_damping_minus_3 = h.cdef_damping_minus_3;
    p.cdef_bits = h.cdef_bits;
    p.cdef_y_strengths = h.cdef_y;
    p.cdef_uv_strengths = h.cdef_uv;
    p.loop_restoration_fields = u16::from(h.lr_type[0])
        | (u16::from(h.lr_type[1]) << 2)
        | (u16::from(h.lr_type[2]) << 4)
        | (u16::from(h.lr_unit_shift) << 6)
        | (u16::from(h.lr_uv_shift) << 8);
    p.wm = [VaWarpedMotionAv1::identity(); 7];
    p
}

/// The reference slots after the frame `h` was decoded into `target` (7.20).
pub(super) fn refresh(slots: &mut [RefSlot; NUM_REF_FRAMES], h: &FrameHeader, target: VaId) {
    for (index, slot) in slots.iter_mut().enumerate() {
        if h.refresh_frame_flags & (1 << index) == 0 {
            continue;
        }
        *slot = RefSlot {
            valid: true,
            surface: target,
            upscaled_width: h.frame_width,
            frame_height: h.frame_height,
            render_width: h.render_width,
            render_height: h.render_height,
            order_hint: h.order_hint,
            ref_deltas: h.ref_deltas,
            mode_deltas: h.mode_deltas,
            feature_enabled: h.segmentation.feature_enabled,
            feature_data: h.segmentation.feature_data,
        };
    }
}

/// The AV1 part of a VA-API decoder of one picture group.
pub(super) struct Av1Units {
    size: FrameSize,
    seq: Option<SequenceHeader>,
    slots: [RefSlot; NUM_REF_FRAMES],
}

impl Av1Units {
    pub(super) fn new(size: FrameSize) -> Self {
        Self {
            size,
            seq: None,
            slots: Default::default(),
        }
    }
}

impl UnitDecoder for Av1Units {
    fn decode_unit(
        &mut self,
        va: &mut VaDecodeSession,
        payload: &[u8],
    ) -> io::Result<Option<VaId>> {
        let mut frame: Option<FrameHeader> = None;
        let mut groups: Vec<TileGroup<'_>> = Vec::new();
        for obu in split_obus(payload)? {
            if obu.spatial_id != 0 {
                return Err(unsupported("AV1 spatial layers are not decoded here"));
            }
            match obu.kind {
                OBU_SEQUENCE_HEADER => {
                    let parsed = parse_sequence_header(obu.payload)?;
                    if self.seq.as_ref() != Some(&parsed) {
                        self.slots = Default::default();
                        self.seq = Some(parsed);
                    }
                }
                OBU_FRAME_HEADER | OBU_FRAME => {
                    if frame.is_some() {
                        return Err(unsupported("AV1 temporal unit with more than one frame"));
                    }
                    let seq = self
                        .seq
                        .as_ref()
                        .ok_or_else(|| invalid("AV1 frame before its sequence header"))?;
                    let h = parse_frame_header(obu.payload, seq, &self.slots, obu.temporal_id, 0)?;
                    if h.frame_width < self.size.width || h.frame_height < self.size.height {
                        return Err(invalid("AV1 frame smaller than its picture group"));
                    }
                    if obu.kind == OBU_FRAME {
                        let rest = obu
                            .payload
                            .get(h.header_bits.div_ceil(8)..)
                            .ok_or_else(|| invalid("truncated AV1 frame OBU"))?;
                        groups.push(parse_tile_group(rest, &h)?);
                    }
                    frame = Some(h);
                }
                OBU_TILE_GROUP => {
                    let h = frame
                        .as_ref()
                        .ok_or_else(|| invalid("AV1 tile group before its frame header"))?;
                    groups.push(parse_tile_group(obu.payload, h)?);
                }
                OBU_TILE_LIST => {
                    return Err(unsupported("AV1 large-scale tiles are not decoded here"))
                }
                // Temporal delimiter, metadata, redundant frame header, padding.
                _ => {}
            }
        }
        let Some(h) = frame else {
            return Ok(None);
        };
        let mut next = 0;
        for group in &groups {
            if group.start != next {
                return Err(invalid("AV1 tile groups out of order"));
            }
            next = group.end + 1;
        }
        if next != h.tiles.count() {
            return Err(invalid("AV1 frame without all of its tiles"));
        }
        let seq = self
            .seq
            .as_ref()
            .ok_or_else(|| invalid("AV1 frame before its sequence header"))?;
        let coded = FrameSize::new(
            seq.max_width.div_ceil(16) * 16,
            seq.max_height.div_ceil(16) * 16,
        );
        if h.intra() {
            if va.context_for(coded, NUM_REF_FRAMES + EXTRA_SURFACES)? {
                self.slots = Default::default();
            }
        } else if va.context.as_ref().is_none_or(|c| c.coded != coded) {
            return Err(invalid("AV1 inter frame without matching VA context"));
        }
        let context = va
            .context
            .as_ref()
            .ok_or_else(|| invalid("no VA context"))?;
        let held = |surface: &VaId| self.slots.iter().any(|s| s.valid && s.surface == *surface);
        let target = *context
            .surfaces
            .iter()
            .find(|surface| !held(surface))
            .ok_or_else(|| invalid("no free VA surface for the frame"))?;
        let mut buffers = Buffers::new(&va.display, context.context);
        let picture = picture_params(seq, &h, &self.slots, target);
        buffers.add(VA_PICTURE_PARAMETER_BUFFER, &picture)?;
        for group in &groups {
            buffers.add_elements(VA_SLICE_PARAMETER_BUFFER, &group.tiles)?;
            buffers.add_bytes(VA_SLICE_DATA_BUFFER, group.data)?;
        }
        buffers.submit(target)?;
        drop(buffers);
        refresh(&mut self.slots, &h, target);
        Ok(Some(target))
    }
}
