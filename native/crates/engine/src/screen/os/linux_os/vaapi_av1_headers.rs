//! The AV1 stream of the VA-API encoder (plan 2026-11-14 „Bildkette“
//! BK4b, B-21 as BK4's H.264): Main profile, 8-bit 4:2:0, a key frame
//! first and on request, otherwise inter frames that predict from the
//! previous frame only (all eight slots refreshed by every frame, all
//! seven references naming slot 0; "compound" with LAST and BWDREF on the
//! same frame where the driver demands both lists, the AV1 form of
//! `VA_PREDICTION_DIRECTION_BI_NOT_EMPTY`), every frame shown, no
//! reordering. The program writes the sequence header OBU (AV1 v1.0.0 with
//! Errata 1, 5.5, BT.709 limited range) and the frame header OBU (5.9) as
//! packed headers where the driver takes them; the frame header carries a
//! four-byte `obu_size` and the bit offsets of `base_q_idx`, the loop
//! filter and CDEF parameters, which the driver's rate control rewrites
//! (libva 2.20.0 `va_enc_av1.h`, `VAEncPictureParameterBufferAV1`).
use super::h264_bits::BitWriter;
use super::hw_encode_common::RateParams;
use super::vaapi_av1_abi::{
    VaEncPictureAv1, VaEncSequenceAv1, VaEncTileGroupAv1, VaWarpedMotionAv1, Zeroed,
};
use super::vaapi_av1_bits::{leb128_fixed4, obu, OBU_FRAME_HEADER, OBU_SEQUENCE_HEADER};
use super::vaapi_av1_syntax::{uniform_sizes, SuperBlocks, TileInfo, DEFAULT_REF_DELTAS};
use super::vaapi_ffi::{
    VaId, VA_ATTRIB_NOT_SUPPORTED, VA_FEATURE_REQUIRED, VA_FEATURE_SUPPORTED,
    VA_PREDICTION_DIRECTION_BI_NOT_EMPTY,
};
use super::vaapi_h264_headers::INTRA_PERIOD;
use crate::screen::core::pixel_convert::FrameSize;

/// `order_hint_bits_minus_1`: 8-bit order hints.
pub(super) const ORDER_HINT_BITS: u32 = 8;
/// `base_q_idx` of the packed header before the rate control rewrites it
/// (never 0, which would drop `delta_q_present_flag` from the syntax).
pub(super) const INITIAL_QINDEX: u8 = 128;
/// Loop filter levels Y vertical/horizontal, U, V the header starts with.
const LOOP_FILTER: [u8; 4] = [8, 8, 8, 8];
/// `cdef_damping_minus_3` (damping 5), one strength pair, all zero.
const CDEF_DAMPING_MINUS_3: u8 = 2;
/// Bits of the OBU header and the four-byte size before the payload.
const OBU_PREFIX_BITS: u32 = 40;

/// The tools of one encoder from `VAConfigAttribEncAV1`, `…Ext1`,
/// `…Ext2` (two bits per tool, `VA_FEATURE_*`) and the prediction
/// direction (assumption A-VA-AV1-ATTRIBS for the defaults without them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Av1Tools {
    pub use_128: bool,
    pub filter_intra: bool,
    pub intra_edge: bool,
    /// Palette for screen content (`seq_force_screen_content_tools` 1).
    pub screen_content: bool,
    pub switchable_filter: bool,
    pub tx_select: bool,
    pub tile_size_bytes: u32,
    /// LAST and BWDREF on the previous frame (`reference_select` 1).
    pub compound: bool,
}

impl Av1Tools {
    pub(super) fn new(features: u32, ext1: u32, ext2: u32, prediction: u32) -> Self {
        let present = |value: u32| value != VA_ATTRIB_NOT_SUPPORTED;
        let field = |shift: u32| {
            if present(features) {
                (features >> shift) & 3
            } else {
                0
            }
        };
        let tx = if present(ext2) { (ext2 >> 4) & 7 } else { 4 };
        Self {
            use_128: field(0) == VA_FEATURE_REQUIRED,
            filter_intra: field(2) >= VA_FEATURE_SUPPORTED,
            intra_edge: field(4) >= VA_FEATURE_SUPPORTED,
            screen_content: field(12) >= VA_FEATURE_SUPPORTED,
            switchable_filter: present(ext1) && ext1 & 0x10 != 0,
            tx_select: tx & 4 != 0 || tx & 2 == 0,
            tile_size_bytes: if present(ext2) { (ext2 & 3) + 1 } else { 4 },
            compound: present(prediction) && prediction & VA_PREDICTION_DIRECTION_BI_NOT_EMPTY != 0,
        }
    }
}

/// Frame geometry of one encoder and its tiles: as few as Annex A allows,
/// uniform (one tile up to 4096 × 2304, the protocol's largest picture).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Av1Layout {
    pub size: FrameSize,
    pub sb: SuperBlocks,
    pub tiles: TileInfo,
}

impl Av1Layout {
    pub(super) fn new(size: FrameSize, tools: &Av1Tools) -> Self {
        let sb = SuperBlocks::new(size.width, size.height, tools.use_128);
        let (min_cols, _, _, min_tiles) = sb.limits();
        let rows_log2 = min_tiles.saturating_sub(min_cols);
        let tiles = TileInfo {
            uniform: true,
            cols_log2: min_cols,
            rows_log2,
            widths: uniform_sizes(sb.cols, min_cols),
            heights: uniform_sizes(sb.rows, rows_log2),
            context_update_tile_id: 0,
            size_bytes: tools.tile_size_bytes as usize,
        };
        Self { size, sb, tiles }
    }

    /// The surface size: whole superblocks (assumption A-VA-AV1-SURFACE).
    pub(super) fn surface(&self) -> FrameSize {
        let side = 1 << self.sb.size_log2;
        FrameSize::new(self.sb.cols * side, self.sb.rows * side)
    }
}

/// The state of one frame to encode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Av1Picture {
    pub key: bool,
    pub order_hint: u8,
    pub recon: VaId,
    /// Reconstructed surface of the previous frame (inter frames).
    pub reference: Option<VaId>,
    pub coded: VaId,
}

/// The sequence header OBU matching `sequence_params`.
pub(super) fn sequence_header(layout: &Av1Layout, tools: &Av1Tools, level: u8) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(3, 0); // seq_profile: Main
    w.flag(false); // still_picture
    w.flag(false); // reduced_still_picture_header
    w.flag(false); // timing_info_present_flag
    w.flag(false); // initial_display_delay_present_flag
    w.bits(5, 0); // operating_points_cnt_minus_1
    w.bits(12, 0); // operating_point_idc[0]
    w.bits(5, u32::from(level)); // seq_level_idx[0]
    if level > 7 {
        w.flag(false); // seq_tier[0]: Main
    }
    w.bits(4, 15); // frame_width_bits_minus_1
    w.bits(4, 15); // frame_height_bits_minus_1
    w.bits(16, layout.size.width - 1);
    w.bits(16, layout.size.height - 1);
    w.flag(false); // frame_id_numbers_present_flag
    w.flag(tools.use_128);
    w.flag(tools.filter_intra);
    w.flag(tools.intra_edge);
    w.flag(false); // enable_interintra_compound
    w.flag(false); // enable_masked_compound
    w.flag(false); // enable_warped_motion
    w.flag(false); // enable_dual_filter
    w.flag(true); // enable_order_hint
    w.flag(false); // enable_jnt_comp
    w.flag(false); // enable_ref_frame_mvs
    w.flag(false); // seq_choose_screen_content_tools
    w.flag(tools.screen_content); // seq_force_screen_content_tools
    if tools.screen_content {
        w.flag(false); // seq_choose_integer_mv
        w.flag(false); // seq_force_integer_mv
    }
    w.bits(3, ORDER_HINT_BITS - 1);
    w.flag(false); // enable_superres
    w.flag(true); // enable_cdef
    w.flag(false); // enable_restoration
                   // color_config(): 8 bit, colour description BT.709, limited range.
    w.flag(false); // high_bitdepth
    w.flag(false); // mono_chrome
    w.flag(true); // color_description_present_flag
    w.bits(8, 1); // color_primaries: BT.709
    w.bits(8, 1); // transfer_characteristics: BT.709
    w.bits(8, 1); // matrix_coefficients: BT.709
    w.flag(false); // color_range: limited
    w.bits(2, 0); // chroma_sample_position: unknown
    w.flag(false); // separate_uv_delta_q
    w.flag(false); // film_grain_params_present
    obu(OBU_SEQUENCE_HEADER, &w.finish())
}

/// A frame header OBU and the offsets VA-API wants of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FrameHeaderObu {
    pub bytes: Vec<u8>,
    pub qindex: u32,
    pub loop_filter: u32,
    pub cdef: u32,
    pub cdef_bits: u32,
    /// Up to and with the trailing one bit.
    pub size_bits: u32,
}

/// The frame header OBU of `picture` (5.9.2), separate from the tile group
/// (`enable_frame_obu` 0).
pub(super) fn frame_header(
    layout: &Av1Layout,
    tools: &Av1Tools,
    picture: &Av1Picture,
) -> FrameHeaderObu {
    let mut w = BitWriter::default();
    let inter = !picture.key;
    w.flag(false); // show_existing_frame
    w.bits(2, u32::from(inter)); // frame_type: KEY 0, INTER 1
    w.flag(true); // show_frame
    if inter {
        w.flag(false); // error_resilient_mode
    }
    w.flag(false); // disable_cdf_update
    w.flag(false); // frame_size_override_flag
    w.bits(ORDER_HINT_BITS, u32::from(picture.order_hint));
    if inter {
        w.bits(3, 0); // primary_ref_frame: LAST
        w.bits(8, 0xff); // refresh_frame_flags
        w.flag(false); // frame_refs_short_signaling
        for _ in 0..7 {
            w.bits(3, 0); // ref_frame_idx: slot 0
        }
        w.flag(false); // render_and_frame_size_different
        w.flag(true); // allow_high_precision_mv
        w.flag(tools.switchable_filter); // is_filter_switchable
        if !tools.switchable_filter {
            w.bits(2, 0); // interpolation_filter: EIGHTTAP
        }
        w.flag(false); // is_motion_mode_switchable
    } else {
        w.flag(false); // render_and_frame_size_different
        if tools.screen_content {
            w.flag(false); // allow_intrabc
        }
    }
    w.flag(false); // disable_frame_end_update_cdf
    write_tile_info(&mut w, layout);
    let qindex = w.position();
    w.bits(8, u32::from(INITIAL_QINDEX)); // base_q_idx
    w.flag(false); // DeltaQYDc
    w.flag(false); // DeltaQUDc
    w.flag(false); // DeltaQUAc
    w.flag(false); // using_qmatrix
    w.flag(false); // segmentation_enabled
    w.flag(false); // delta_q_present
    let loop_filter = w.position();
    for level in LOOP_FILTER {
        w.bits(6, u32::from(level));
    }
    w.bits(3, 0); // loop_filter_sharpness
    w.flag(false); // loop_filter_delta_enabled
    let cdef = w.position();
    w.bits(2, u32::from(CDEF_DAMPING_MINUS_3));
    w.bits(2, 0); // cdef_bits
    w.bits(4, 0); // cdef_y_pri_strength[0]
    w.bits(2, 0); // cdef_y_sec_strength[0]
    w.bits(4, 0); // cdef_uv_pri_strength[0]
    w.bits(2, 0); // cdef_uv_sec_strength[0]
    let cdef_bits = w.position() - cdef;
    w.flag(tools.tx_select); // tx_mode_select
    if inter {
        w.flag(tools.compound); // reference_select
    }
    w.flag(false); // reduced_tx_set
    if inter {
        for _ in 0..7 {
            w.flag(false); // is_global
        }
    }
    let size_bits = OBU_PREFIX_BITS + w.position() + 1;
    let payload = w.finish();
    let mut bytes = vec![(OBU_FRAME_HEADER << 3) | 0x02];
    bytes.extend_from_slice(&leb128_fixed4(payload.len() as u32));
    bytes.extend_from_slice(&payload);
    FrameHeaderObu {
        bytes,
        qindex: OBU_PREFIX_BITS + qindex,
        loop_filter: OBU_PREFIX_BITS + loop_filter,
        cdef: OBU_PREFIX_BITS + cdef,
        cdef_bits,
        size_bits,
    }
}

/// `tile_info()` of the uniform minimal layout (5.9.15).
fn write_tile_info(w: &mut BitWriter, layout: &Av1Layout) {
    let (min_cols, max_cols, max_rows, min_tiles) = layout.sb.limits();
    let tiles = &layout.tiles;
    w.flag(true); // uniform_tile_spacing_flag
    if tiles.cols_log2 < max_cols {
        w.flag(false); // increment_tile_cols_log2
    }
    if min_tiles.saturating_sub(min_cols) < max_rows {
        w.flag(false); // increment_tile_rows_log2
    }
    if tiles.cols_log2 > 0 || tiles.rows_log2 > 0 {
        w.bits(tiles.cols_log2 + tiles.rows_log2, 0); // context_update_tile_id
        w.bits(2, tiles.size_bytes as u32 - 1); // tile_size_bytes_minus_1
    }
}

/// The sequence parameter buffer (fields as `sequence_header`).
pub(super) fn sequence_params(tools: &Av1Tools, level: u8, rate: &RateParams) -> VaEncSequenceAv1 {
    let mut seq = VaEncSequenceAv1::zeroed();
    seq.seq_level_idx = level;
    seq.intra_period = INTRA_PERIOD;
    seq.ip_period = 1;
    seq.bits_per_second = rate.bits_per_second;
    seq.seq_fields = (u32::from(tools.use_128) << 1)
        | (u32::from(tools.filter_intra) << 2)
        | (u32::from(tools.intra_edge) << 3)
        | (1 << 8) // enable_order_hint
        | (1 << 12) // enable_cdef
        | (1 << 17) // subsampling_x
        | (1 << 18); // subsampling_y
    seq.order_hint_bits_minus_1 = (ORDER_HINT_BITS - 1) as u8;
    seq
}

/// The picture parameter buffer of `picture` with the offsets of `header`.
pub(super) fn picture_params(
    layout: &Av1Layout,
    tools: &Av1Tools,
    picture: &Av1Picture,
    header: &FrameHeaderObu,
) -> VaEncPictureAv1 {
    let mut out = VaEncPictureAv1::zeroed();
    out.frame_width_minus_1 = (layout.size.width - 1) as u16;
    out.frame_height_minus_1 = (layout.size.height - 1) as u16;
    out.reconstructed_frame = picture.recon;
    out.coded_buf = picture.coded;
    let reference = picture.reference.filter(|_| !picture.key);
    out.reference_frames = [reference.unwrap_or(super::vaapi_ffi::VA_INVALID_ID); 8];
    out.primary_ref_frame = if picture.key { 7 } else { 0 };
    out.order_hint = picture.order_hint;
    out.refresh_frame_flags = 0xff;
    if !picture.key {
        out.ref_frame_ctrl_l0 = 1; // search_idx0: LAST
        if tools.compound {
            out.ref_frame_ctrl_l1 = 5; // search_idx0: BWDREF
        }
    }
    out.picture_flags = u32::from(!picture.key) // frame_type
        | (u32::from(picture.key) << 2) // error_resilient_mode
        | (u32::from(!picture.key) << 5) // allow_high_precision_mv
        | (u32::from(tools.screen_content) << 13); // palette_mode_enable
    out.filter_level = [LOOP_FILTER[0], LOOP_FILTER[1]];
    out.filter_level_u = LOOP_FILTER[2];
    out.filter_level_v = LOOP_FILTER[3];
    out.interpolation_filter = if tools.switchable_filter { 4 } else { 0 };
    out.ref_deltas = DEFAULT_REF_DELTAS;
    out.base_qindex = INITIAL_QINDEX;
    out.min_base_qindex = 1;
    out.max_base_qindex = 255;
    let tx_mode = if tools.tx_select { 2 } else { 1 };
    let reference_mode = if tools.compound && !picture.key { 2 } else { 0 };
    out.mode_control_flags = (tx_mode << 7) | (reference_mode << 9);
    out.tile_cols = layout.tiles.widths.len() as u8;
    out.tile_rows = layout.tiles.heights.len() as u8;
    for (slot, width) in out
        .width_in_sbs_minus_1
        .iter_mut()
        .zip(&layout.tiles.widths)
    {
        *slot = (*width - 1) as u16;
    }
    for (slot, height) in out
        .height_in_sbs_minus_1
        .iter_mut()
        .zip(&layout.tiles.heights)
    {
        *slot = (*height - 1) as u16;
    }
    out.cdef_damping_minus_3 = CDEF_DAMPING_MINUS_3;
    out.wm = [VaWarpedMotionAv1::identity(); 7];
    out.bit_offset_qindex = header.qindex;
    out.bit_offset_loopfilter_params = header.loop_filter;
    out.bit_offset_cdef_params = header.cdef;
    out.size_in_bits_cdef_params = header.cdef_bits;
    out.byte_offset_frame_hdr_obu_size = 1;
    out.size_in_bits_frame_hdr_obu = header.size_bits;
    out.tile_group_obu_hdr_info = 0x02; // obu_has_size_field
    out
}

/// The one tile group: every tile.
pub(super) fn tile_group(layout: &Av1Layout) -> VaEncTileGroupAv1 {
    VaEncTileGroupAv1 {
        tg_start: 0,
        tg_end: (layout.tiles.count() - 1) as u8,
        ..VaEncTileGroupAv1::default()
    }
}
