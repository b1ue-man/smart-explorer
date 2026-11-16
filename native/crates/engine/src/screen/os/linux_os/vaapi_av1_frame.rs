//! AV1 frame header for the VA-API AV1 decoder (plan 2026-11-14
//! „Bildkette“ BK4b, B-38): `uncompressed_header()` of AV1 v1.0.0 with
//! Errata 1, 5.9.2 to 5.9.25, as far as VA-API needs it
//! (`VADecPictureParameterBufferAV1`; the GPU parses the tiles). Taken:
//! key and inter frames that are shown at once, every coding tool whose
//! parameters VA-API carries (tiles, quantizer matrices, segmentation,
//! delta Q/LF, loop filter, CDEF, loop restoration, TX mode, compound
//! reference, skip mode, warped motion, reduced TX set). Refused with
//! `ErrorKind::Unsupported`, so the next decoder of the register opens:
//! `show_existing_frame`, hidden frames (both reorder pictures), intra-only
//! and switch frames, short reference signalling, superres, global motion,
//! large-scale tiles. None of them is written by the encoders of this
//! program (BK4b VA-API, NVENC, SVT-AV1 in real-time low delay) with the
//! settings of the project (assumption A-AV1-TOOLS).
use std::io;

use super::h264_bits::BitReader;
use super::vaapi_av1_bits::{delta_q, su};
use super::vaapi_av1_syntax::{
    parse_tile_info, unsupported, RefSlot, SequenceHeader, SuperBlocks, TileInfo,
    DEFAULT_REF_DELTAS, NUM_REF_FRAMES, PRIMARY_REF_NONE, REFS_PER_FRAME, SELECT,
};
use crate::screen::core::wire::invalid;

pub(super) const KEY_FRAME: u8 = 0;
pub(super) const INTER_FRAME: u8 = 1;
/// `Segmentation_Feature_Bits`, `_Signed`, `_Max` (5.9.14).
const FEATURE_BITS: [u32; 8] = [8, 6, 6, 6, 6, 3, 0, 0];
const FEATURE_SIGNED: [bool; 8] = [true, true, true, true, true, false, false, false];
const FEATURE_MAX: [i32; 8] = [255, 63, 63, 63, 63, 7, 0, 0];

/// Segmentation of the frame (5.9.14).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Segmentation {
    pub enabled: bool,
    pub update_map: bool,
    pub temporal_update: bool,
    pub update_data: bool,
    pub feature_enabled: [[bool; 8]; 8],
    pub feature_data: [[i16; 8]; 8],
}

/// The parsed frame header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct FrameHeader {
    pub frame_type: u8,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub error_resilient: bool,
    pub disable_cdf_update: bool,
    pub allow_screen_content_tools: bool,
    pub force_integer_mv: bool,
    pub order_hint: u32,
    pub primary_ref_frame: u8,
    pub refresh_frame_flags: u8,
    pub ref_frame_idx: [u8; REFS_PER_FRAME],
    pub frame_width: u32,
    pub frame_height: u32,
    pub render_width: u32,
    pub render_height: u32,
    pub allow_intrabc: bool,
    pub allow_high_precision_mv: bool,
    pub interpolation_filter: u8,
    pub is_motion_mode_switchable: bool,
    pub use_ref_frame_mvs: bool,
    pub disable_frame_end_update_cdf: bool,
    pub tiles: TileInfo,
    pub base_q_idx: u8,
    /// DeltaQYDc, DeltaQUDc, DeltaQUAc, DeltaQVDc, DeltaQVAc.
    pub delta_q: [i8; 5],
    /// `using_qmatrix`, `qm_y`, `qm_u`, `qm_v`.
    pub qmatrix: (bool, u8, u8, u8),
    pub segmentation: Segmentation,
    pub delta_q_present: bool,
    pub delta_q_res: u8,
    pub delta_lf_present: bool,
    pub delta_lf_res: u8,
    pub delta_lf_multi: bool,
    pub coded_lossless: bool,
    pub loop_filter_level: [u8; 4],
    pub sharpness: u8,
    pub delta_enabled: bool,
    pub delta_update: bool,
    pub ref_deltas: [i8; 8],
    pub mode_deltas: [i8; 2],
    pub cdef_damping_minus_3: u8,
    pub cdef_bits: u8,
    /// `pri << 2 | sec` as coded (va_dec_av1.h).
    pub cdef_y: [u8; 8],
    pub cdef_uv: [u8; 8],
    /// `FrameRestorationType` of Y, U, V after `Remap_Lr_Type`.
    pub lr_type: [u8; 3],
    pub lr_unit_shift: u8,
    pub lr_uv_shift: u8,
    pub tx_mode: u8,
    pub reference_select: bool,
    pub skip_mode_present: bool,
    pub allow_warped_motion: bool,
    pub reduced_tx_set: bool,
    /// Bits of `uncompressed_header()` (the tile group of an OBU_FRAME
    /// starts at the next byte).
    pub header_bits: usize,
}

impl FrameHeader {
    pub(super) fn intra(&self) -> bool {
        self.frame_type == KEY_FRAME
    }
}

/// `get_relative_dist` (7.12.3).
fn relative_dist(seq: &SequenceHeader, a: u32, b: u32) -> i32 {
    if !seq.enable_order_hint {
        return 0;
    }
    let diff = a as i32 - b as i32;
    let m = 1i32 << (seq.order_hint_bits - 1);
    (diff & (m - 1)) - (diff & m)
}

/// Parses `uncompressed_header()` from a frame header or frame OBU payload.
pub(super) fn parse_frame_header(
    payload: &[u8],
    seq: &SequenceHeader,
    slots: &[RefSlot; NUM_REF_FRAMES],
    temporal_id: u8,
    spatial_id: u8,
) -> io::Result<FrameHeader> {
    let mut r = BitReader::new(payload);
    let mut h = FrameHeader::default();
    if r.flag()? {
        return Err(unsupported("AV1 show_existing_frame reorders pictures"));
    }
    h.frame_type = r.bits(2)? as u8;
    if !matches!(h.frame_type, KEY_FRAME | INTER_FRAME) {
        return Err(unsupported(
            "AV1 intra-only and switch frames are not decoded here",
        ));
    }
    h.show_frame = r.flag()?;
    if !h.show_frame {
        return Err(unsupported("AV1 hidden frames reorder pictures"));
    }
    if seq.decoder_model_info_present && !seq.equal_picture_interval {
        r.bits(seq.frame_presentation_time_length)?; // temporal_point_info
    }
    h.showable_frame = h.frame_type != KEY_FRAME;
    h.error_resilient = h.frame_type == KEY_FRAME || r.flag()?;
    h.disable_cdf_update = r.flag()?;
    h.allow_screen_content_tools = if seq.seq_force_screen_content_tools == SELECT {
        r.flag()?
    } else {
        seq.seq_force_screen_content_tools == 1
    };
    h.force_integer_mv = if !h.allow_screen_content_tools {
        false
    } else if seq.seq_force_integer_mv == SELECT {
        r.flag()?
    } else {
        seq.seq_force_integer_mv == 1
    };
    if h.intra() {
        h.force_integer_mv = true;
    }
    if seq.frame_id_numbers_present {
        r.bits(seq.frame_id_length)?; // current_frame_id
    }
    let frame_size_override = r.flag()?;
    h.order_hint = r.bits(seq.order_hint_bits)?;
    h.primary_ref_frame = if h.intra() || h.error_resilient {
        PRIMARY_REF_NONE
    } else {
        r.bits(3)? as u8
    };
    if seq.decoder_model_info_present && r.flag()? {
        for (idc, model) in &seq.operating_points {
            let in_temporal = (idc >> temporal_id) & 1 == 1;
            let in_spatial = (idc >> (u32::from(spatial_id) + 8)) & 1 == 1;
            if *model && (*idc == 0 || (in_temporal && in_spatial)) {
                r.bits(seq.buffer_removal_time_length)?;
            }
        }
    }
    h.refresh_frame_flags = if h.intra() { 0xff } else { r.bits(8)? as u8 };
    if !h.intra() && h.error_resilient && seq.enable_order_hint {
        for _ in 0..NUM_REF_FRAMES {
            r.bits(seq.order_hint_bits)?; // ref_order_hint
        }
    }
    if h.intra() {
        frame_size(&mut r, seq, frame_size_override, &mut h)?;
        render_size(&mut r, &mut h)?;
        if h.allow_screen_content_tools {
            h.allow_intrabc = r.flag()?;
        }
    } else {
        inter_header(&mut r, seq, slots, frame_size_override, &mut h)?;
    }
    h.disable_frame_end_update_cdf = h.disable_cdf_update || r.flag()?;
    // setup_past_independence or load_previous (7.20, 7.21).
    let previous = (h.primary_ref_frame != PRIMARY_REF_NONE)
        .then(|| &slots[h.ref_frame_idx[usize::from(h.primary_ref_frame)] as usize]);
    h.ref_deltas = previous.map_or(DEFAULT_REF_DELTAS, |slot| slot.ref_deltas);
    h.mode_deltas = previous.map_or([0; 2], |slot| slot.mode_deltas);
    if let Some(slot) = previous {
        h.segmentation.feature_enabled = slot.feature_enabled;
        h.segmentation.feature_data = slot.feature_data;
    }
    let sb = SuperBlocks::new(h.frame_width, h.frame_height, seq.use_128x128_superblock);
    h.tiles = parse_tile_info(&mut r, sb)?;
    quantization(&mut r, seq, &mut h)?;
    segmentation(&mut r, &mut h)?;
    delta_params(&mut r, &mut h)?;
    h.coded_lossless =
        (0..8).all(|segment| h.delta_q.iter().all(|delta| *delta == 0) && qindex(&h, segment) == 0);
    super::vaapi_av1_filters::loop_filter(&mut r, &mut h)?;
    super::vaapi_av1_filters::cdef(&mut r, seq, &mut h)?;
    super::vaapi_av1_filters::restoration(&mut r, seq, &mut h)?;
    h.tx_mode = if h.coded_lossless {
        0
    } else if r.flag()? {
        2
    } else {
        1
    };
    if !h.intra() {
        h.reference_select = r.flag()?;
    }
    if skip_mode_allowed(seq, slots, &h) {
        h.skip_mode_present = r.flag()?;
    }
    if !h.intra() && !h.error_resilient && seq.enable_warped_motion {
        h.allow_warped_motion = r.flag()?;
    }
    h.reduced_tx_set = r.flag()?;
    if !h.intra() {
        for _ in 0..REFS_PER_FRAME {
            if r.flag()? {
                return Err(unsupported("AV1 global motion is not decoded here"));
            }
        }
    }
    h.header_bits = r.position();
    Ok(h)
}

/// The inter part of `uncompressed_header()`.
fn inter_header(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    slots: &[RefSlot; NUM_REF_FRAMES],
    frame_size_override: bool,
    h: &mut FrameHeader,
) -> io::Result<()> {
    if seq.enable_order_hint && r.flag()? {
        return Err(unsupported(
            "AV1 short reference signalling is not decoded here",
        ));
    }
    for index in 0..REFS_PER_FRAME {
        let slot = r.bits(3)? as u8;
        if !slots[slot as usize].valid {
            return Err(invalid("AV1 frame references an empty slot"));
        }
        h.ref_frame_idx[index] = slot;
        if seq.frame_id_numbers_present {
            r.bits(seq.delta_frame_id_length)?; // delta_frame_id_minus_1
        }
    }
    if frame_size_override && !h.error_resilient {
        // frame_size_with_refs() (5.9.7).
        let mut found = None;
        for index in 0..REFS_PER_FRAME {
            if r.flag()? {
                found = Some(&slots[h.ref_frame_idx[index] as usize]);
                break;
            }
        }
        match found {
            Some(slot) => {
                h.frame_width = slot.upscaled_width;
                h.frame_height = slot.frame_height;
                h.render_width = slot.render_width;
                h.render_height = slot.render_height;
                superres(r, seq)?;
            }
            None => {
                frame_size(r, seq, true, h)?;
                render_size(r, h)?;
            }
        }
    } else {
        frame_size(r, seq, frame_size_override, h)?;
        render_size(r, h)?;
    }
    h.allow_high_precision_mv = !h.force_integer_mv && r.flag()?;
    h.interpolation_filter = if r.flag()? { 4 } else { r.bits(2)? as u8 };
    h.is_motion_mode_switchable = r.flag()?;
    h.use_ref_frame_mvs = !h.error_resilient && seq.enable_ref_frame_mvs && r.flag()?;
    Ok(())
}

/// `frame_size()` (5.9.5) with `superres_params()`.
fn frame_size(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    override_flag: bool,
    h: &mut FrameHeader,
) -> io::Result<()> {
    if override_flag {
        h.frame_width = r.bits(seq.frame_width_bits)? + 1;
        h.frame_height = r.bits(seq.frame_height_bits)? + 1;
    } else {
        h.frame_width = seq.max_width;
        h.frame_height = seq.max_height;
    }
    if h.frame_width > seq.max_width || h.frame_height > seq.max_height {
        return Err(invalid("AV1 frame larger than its sequence header"));
    }
    superres(r, seq)
}

/// `superres_params()` (5.9.8); superres itself is refused.
fn superres(r: &mut BitReader<'_>, seq: &SequenceHeader) -> io::Result<()> {
    if seq.enable_superres && r.flag()? {
        return Err(unsupported("AV1 superres is not decoded here"));
    }
    Ok(())
}

/// `render_size()` (5.9.6).
fn render_size(r: &mut BitReader<'_>, h: &mut FrameHeader) -> io::Result<()> {
    if r.flag()? {
        h.render_width = r.bits(16)? + 1;
        h.render_height = r.bits(16)? + 1;
    } else {
        h.render_width = h.frame_width;
        h.render_height = h.frame_height;
    }
    Ok(())
}

/// `quantization_params()` (5.9.12).
fn quantization(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    h: &mut FrameHeader,
) -> io::Result<()> {
    h.base_q_idx = r.bits(8)? as u8;
    let y_dc = delta_q(r)?;
    let diff_uv_delta = seq.separate_uv_delta_q && r.flag()?;
    let (u_dc, u_ac) = (delta_q(r)?, delta_q(r)?);
    let (v_dc, v_ac) = if diff_uv_delta {
        (delta_q(r)?, delta_q(r)?)
    } else {
        (u_dc, u_ac)
    };
    h.delta_q = [y_dc, u_dc, u_ac, v_dc, v_ac].map(|value| value as i8);
    if r.flag()? {
        let qm_y = r.bits(4)? as u8;
        let qm_u = r.bits(4)? as u8;
        let qm_v = if seq.separate_uv_delta_q {
            r.bits(4)? as u8
        } else {
            qm_u
        };
        h.qmatrix = (true, qm_y, qm_u, qm_v);
    }
    Ok(())
}

/// `segmentation_params()` (5.9.14); the data of the primary reference
/// is already in `h.segmentation`.
fn segmentation(r: &mut BitReader<'_>, h: &mut FrameHeader) -> io::Result<()> {
    let seg = &mut h.segmentation;
    seg.enabled = r.flag()?;
    if !seg.enabled {
        seg.feature_enabled = [[false; 8]; 8];
        seg.feature_data = [[0; 8]; 8];
        return Ok(());
    }
    if h.primary_ref_frame == PRIMARY_REF_NONE {
        seg.update_map = true;
        seg.update_data = true;
    } else {
        seg.update_map = r.flag()?;
        seg.temporal_update = seg.update_map && r.flag()?;
        seg.update_data = r.flag()?;
    }
    if seg.update_data {
        for segment in 0..8 {
            for feature in 0..8 {
                let enabled = r.flag()?;
                let mut value = 0i32;
                if enabled {
                    let (bits, max) = (FEATURE_BITS[feature], FEATURE_MAX[feature]);
                    value = if FEATURE_SIGNED[feature] {
                        su(r, 1 + bits)?.clamp(-max, max)
                    } else {
                        (r.bits(bits)? as i32).clamp(0, max)
                    };
                }
                seg.feature_enabled[segment][feature] = enabled;
                seg.feature_data[segment][feature] = value as i16;
            }
        }
    }
    Ok(())
}

/// `delta_q_params()` and `delta_lf_params()` (5.9.17, 5.9.18).
fn delta_params(r: &mut BitReader<'_>, h: &mut FrameHeader) -> io::Result<()> {
    if h.base_q_idx > 0 {
        h.delta_q_present = r.flag()?;
    }
    if h.delta_q_present {
        h.delta_q_res = r.bits(2)? as u8;
        if !h.allow_intrabc {
            h.delta_lf_present = r.flag()?;
        }
        if h.delta_lf_present {
            h.delta_lf_res = r.bits(2)? as u8;
            h.delta_lf_multi = r.flag()?;
        }
    }
    Ok(())
}

/// `get_qindex(1, segment)` (7.12.2).
pub(super) fn qindex(h: &FrameHeader, segment: usize) -> i32 {
    let seg = &h.segmentation;
    let base = i32::from(h.base_q_idx);
    if seg.enabled && seg.feature_enabled[segment][0] {
        (base + i32::from(seg.feature_data[segment][0])).clamp(0, 255)
    } else {
        base
    }
}

/// `skipModeAllowed` of `skip_mode_params()` (5.9.22).
fn skip_mode_allowed(seq: &SequenceHeader, slots: &[RefSlot; 8], h: &FrameHeader) -> bool {
    if h.intra() || !h.reference_select || !seq.enable_order_hint {
        return false;
    }
    let hint = |index: usize| slots[h.ref_frame_idx[index] as usize].order_hint;
    let (mut forward, mut backward): (Option<u32>, Option<u32>) = (None, None);
    for index in 0..REFS_PER_FRAME {
        let ref_hint = hint(index);
        let dist = relative_dist(seq, ref_hint, h.order_hint);
        if dist < 0 {
            if forward.is_none_or(|f| relative_dist(seq, ref_hint, f) > 0) {
                forward = Some(ref_hint);
            }
        } else if dist > 0 && backward.is_none_or(|b| relative_dist(seq, ref_hint, b) < 0) {
            backward = Some(ref_hint);
        }
    }
    let Some(forward) = forward else {
        return false;
    };
    if backward.is_some() {
        return true;
    }
    (0..REFS_PER_FRAME).any(|index| relative_dist(seq, hint(index), forward) < 0)
}
