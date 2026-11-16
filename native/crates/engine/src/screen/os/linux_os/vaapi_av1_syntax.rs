//! AV1 sequence header and tile layout for the VA-API AV1 coders (plan
//! 2026-11-14 „Bildkette“ BK4b): `sequence_header_obu()` with
//! `color_config()` and the operating points (AV1 v1.0.0 with Errata 1,
//! 5.5), the tile layout of `tile_info()` (5.9.15 with the limits of
//! Annex A.3: MAX_TILE_WIDTH 4096, MAX_TILE_AREA 4096 × 2304, MAX_TILE_COLS
//! and MAX_TILE_ROWS 64) shared by the encoder (which writes it) and the
//! decoder (which reads it), and the state the decoder keeps per reference
//! slot (7.20). The screen contract is profile 0 (Main), 8 bit, 4:2:0;
//! anything else is refused with `ErrorKind::Unsupported`, so the next
//! decoder of the register opens instead.
use std::io;

use super::h264_bits::BitReader;
use super::vaapi_av1_bits::{tile_log2, uvlc};
use crate::screen::core::wire::invalid;

pub(super) const NUM_REF_FRAMES: usize = 8;
pub(super) const REFS_PER_FRAME: usize = 7;
pub(super) const PRIMARY_REF_NONE: u8 = 7;
/// `seq_force_screen_content_tools`/`seq_force_integer_mv` "select".
pub(super) const SELECT: u8 = 2;
/// Annex A.3 tile limits.
const MAX_TILE_WIDTH: u32 = 4096;
const MAX_TILE_AREA: u32 = 4096 * 2304;
const MAX_TILE_COLS: u32 = 64;
const MAX_TILE_ROWS: u32 = 64;

pub(super) fn unsupported(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, what)
}

/// The parts of a sequence header the coders use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SequenceHeader {
    pub frame_width_bits: u32,
    pub frame_height_bits: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub decoder_model_info_present: bool,
    pub equal_picture_interval: bool,
    pub buffer_removal_time_length: u32,
    pub frame_presentation_time_length: u32,
    /// `operating_point_idc` and `decoder_model_present_for_this_op`.
    pub operating_points: Vec<(u32, bool)>,
    pub frame_id_numbers_present: bool,
    pub delta_frame_id_length: u32,
    pub frame_id_length: u32,
    pub use_128x128_superblock: bool,
    pub enable_filter_intra: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_interintra_compound: bool,
    pub enable_masked_compound: bool,
    pub enable_warped_motion: bool,
    pub enable_dual_filter: bool,
    pub enable_order_hint: bool,
    pub enable_jnt_comp: bool,
    pub enable_ref_frame_mvs: bool,
    pub seq_force_screen_content_tools: u8,
    pub seq_force_integer_mv: u8,
    pub order_hint_bits: u32,
    pub enable_superres: bool,
    pub enable_cdef: bool,
    pub enable_restoration: bool,
    pub color_range: bool,
    pub matrix_coefficients: u8,
    pub chroma_sample_position: u8,
    pub separate_uv_delta_q: bool,
}

/// Parses a sequence header OBU payload; refuses everything outside
/// profile 0, 8 bit, 4:2:0, film grain and the reduced still picture header.
pub(super) fn parse_sequence_header(payload: &[u8]) -> io::Result<SequenceHeader> {
    let mut r = BitReader::new(payload);
    let mut seq = SequenceHeader::default();
    if r.bits(3)? != 0 {
        return Err(unsupported("AV1 profile other than Main"));
    }
    r.bits(1)?; // still_picture
    if r.flag()? {
        return Err(unsupported("AV1 reduced still picture header"));
    }
    let mut buffer_delay_length = 0;
    if r.flag()? {
        // timing_info()
        r.bits(32)?; // num_units_in_display_tick
        r.bits(32)?; // time_scale
        seq.equal_picture_interval = r.flag()?;
        if seq.equal_picture_interval {
            uvlc(&mut r)?; // num_ticks_per_picture_minus_1
        }
        seq.decoder_model_info_present = r.flag()?;
        if seq.decoder_model_info_present {
            buffer_delay_length = r.bits(5)? + 1;
            r.bits(32)?; // num_units_in_decoding_tick
            seq.buffer_removal_time_length = r.bits(5)? + 1;
            seq.frame_presentation_time_length = r.bits(5)? + 1;
        }
    }
    let initial_display_delay_present = r.flag()?;
    let points = r.bits(5)? + 1;
    for _ in 0..points {
        let idc = r.bits(12)?;
        // seq_level_idx (the core's `check_av1_sequence` checks it).
        if r.bits(5)? > 7 {
            r.bits(1)?; // seq_tier
        }
        let mut model = false;
        if seq.decoder_model_info_present {
            model = r.flag()?;
            if model {
                r.bits(buffer_delay_length)?; // decoder_buffer_delay
                r.bits(buffer_delay_length)?; // encoder_buffer_delay
                r.bits(1)?; // low_delay_mode_flag
            }
        }
        if initial_display_delay_present && r.flag()? {
            r.bits(4)?; // initial_display_delay_minus_1
        }
        seq.operating_points.push((idc, model));
    }
    seq.frame_width_bits = r.bits(4)? + 1;
    seq.frame_height_bits = r.bits(4)? + 1;
    seq.max_width = r.bits(seq.frame_width_bits)? + 1;
    seq.max_height = r.bits(seq.frame_height_bits)? + 1;
    seq.frame_id_numbers_present = r.flag()?;
    if seq.frame_id_numbers_present {
        seq.delta_frame_id_length = r.bits(4)? + 2;
        seq.frame_id_length = r.bits(3)? + 1 + seq.delta_frame_id_length;
    }
    seq.use_128x128_superblock = r.flag()?;
    seq.enable_filter_intra = r.flag()?;
    seq.enable_intra_edge_filter = r.flag()?;
    seq.enable_interintra_compound = r.flag()?;
    seq.enable_masked_compound = r.flag()?;
    seq.enable_warped_motion = r.flag()?;
    seq.enable_dual_filter = r.flag()?;
    seq.enable_order_hint = r.flag()?;
    if seq.enable_order_hint {
        seq.enable_jnt_comp = r.flag()?;
        seq.enable_ref_frame_mvs = r.flag()?;
    }
    seq.seq_force_screen_content_tools = if r.flag()? { SELECT } else { r.bits(1)? as u8 };
    seq.seq_force_integer_mv = if seq.seq_force_screen_content_tools > 0 {
        if r.flag()? {
            SELECT
        } else {
            r.bits(1)? as u8
        }
    } else {
        SELECT
    };
    if seq.enable_order_hint {
        seq.order_hint_bits = r.bits(3)? + 1;
    }
    seq.enable_superres = r.flag()?;
    seq.enable_cdef = r.flag()?;
    seq.enable_restoration = r.flag()?;
    color_config(&mut r, &mut seq)?;
    if r.flag()? {
        return Err(unsupported("AV1 film grain is outside the screen contract"));
    }
    Ok(seq)
}

/// `color_config()` (5.5.2) of profile 0.
fn color_config(r: &mut BitReader<'_>, seq: &mut SequenceHeader) -> io::Result<()> {
    if r.flag()? {
        return Err(unsupported(
            "AV1 above 8 bits is outside the screen contract",
        ));
    }
    if r.flag()? {
        return Err(unsupported("AV1 monochrome is outside the screen contract"));
    }
    let (mut primaries, mut transfer, mut matrix) = (2, 2, 2);
    if r.flag()? {
        primaries = r.bits(8)?;
        transfer = r.bits(8)?;
        matrix = r.bits(8)?;
    }
    if primaries == 1 && transfer == 13 && matrix == 0 {
        // sRGB implies 4:4:4, which profile 0 does not carry.
        return Err(unsupported("AV1 4:4:4 sRGB is outside the screen contract"));
    }
    seq.matrix_coefficients = matrix as u8;
    seq.color_range = r.flag()?;
    seq.chroma_sample_position = r.bits(2)? as u8;
    seq.separate_uv_delta_q = r.flag()?;
    Ok(())
}

/// Superblock geometry of a frame size (5.9.15, 7.3: `MiCols` in 4 × 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SuperBlocks {
    pub cols: u32,
    pub rows: u32,
    /// log2 of the superblock side in samples (6 or 7).
    pub size_log2: u32,
}

impl SuperBlocks {
    pub(super) fn new(width: u32, height: u32, use_128: bool) -> Self {
        let mi_cols = 2 * width.div_ceil(8);
        let mi_rows = 2 * height.div_ceil(8);
        let (shift, size_log2) = if use_128 { (5, 7) } else { (4, 6) };
        let round = (1 << shift) - 1;
        Self {
            cols: (mi_cols + round) >> shift,
            rows: (mi_rows + round) >> shift,
            size_log2,
        }
    }

    /// `minLog2TileCols`, `maxLog2TileCols`, `maxLog2TileRows`,
    /// `minLog2Tiles`.
    pub(super) fn limits(&self) -> (u32, u32, u32, u32) {
        let max_width_sb = MAX_TILE_WIDTH >> self.size_log2;
        let max_area_sb = MAX_TILE_AREA >> (2 * self.size_log2);
        let min_cols = tile_log2(max_width_sb, self.cols);
        let max_cols = tile_log2(1, self.cols.min(MAX_TILE_COLS));
        let max_rows = tile_log2(1, self.rows.min(MAX_TILE_ROWS));
        let min_tiles = min_cols.max(tile_log2(max_area_sb, self.rows * self.cols));
        (min_cols, max_cols, max_rows, min_tiles)
    }
}

/// The tiles of a frame (5.9.15).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TileInfo {
    pub uniform: bool,
    pub cols_log2: u32,
    pub rows_log2: u32,
    /// Width and height of each tile column and row in superblocks.
    pub widths: Vec<u32>,
    pub heights: Vec<u32>,
    pub context_update_tile_id: u32,
    /// `TileSizeBytes`.
    pub size_bytes: usize,
}

impl TileInfo {
    pub(super) fn count(&self) -> usize {
        self.widths.len() * self.heights.len()
    }
}

/// Uniform tile sizes of `sbs` superblocks in `1 << log2` parts.
pub(super) fn uniform_sizes(sbs: u32, log2: u32) -> Vec<u32> {
    let size = (sbs + (1 << log2) - 1) >> log2;
    let mut out = Vec::new();
    let mut start = 0;
    while start < sbs {
        out.push(size.min(sbs - start));
        start += size;
    }
    out
}

/// Reads `tile_info()` for `sb`.
pub(super) fn parse_tile_info(r: &mut BitReader<'_>, sb: SuperBlocks) -> io::Result<TileInfo> {
    let (min_cols, max_cols, max_rows, min_tiles) = sb.limits();
    let mut info = TileInfo {
        uniform: r.flag()?,
        ..TileInfo::default()
    };
    if info.uniform {
        info.cols_log2 = min_cols;
        while info.cols_log2 < max_cols && r.flag()? {
            info.cols_log2 += 1;
        }
        info.widths = uniform_sizes(sb.cols, info.cols_log2);
        info.rows_log2 = min_tiles.saturating_sub(info.cols_log2);
        while info.rows_log2 < max_rows && r.flag()? {
            info.rows_log2 += 1;
        }
        info.heights = uniform_sizes(sb.rows, info.rows_log2);
    } else {
        let mut widest = 1;
        let mut start = 0;
        while start < sb.cols {
            let max_width = (sb.cols - start).min(MAX_TILE_WIDTH >> sb.size_log2);
            let width = super::vaapi_av1_bits::ns(r, max_width)? + 1;
            widest = widest.max(width);
            info.widths.push(width);
            start += width;
        }
        info.cols_log2 = tile_log2(1, info.widths.len() as u32);
        let max_tile_area_sb = if min_tiles > 0 {
            (sb.rows * sb.cols) >> (min_tiles + 1)
        } else {
            sb.rows * sb.cols
        };
        let max_height = (max_tile_area_sb / widest).max(1);
        start = 0;
        while start < sb.rows {
            let limit = (sb.rows - start).min(max_height);
            let height = super::vaapi_av1_bits::ns(r, limit)? + 1;
            info.heights.push(height);
            start += height;
        }
        info.rows_log2 = tile_log2(1, info.heights.len() as u32);
    }
    if info.widths.len() > MAX_TILE_COLS as usize || info.heights.len() > MAX_TILE_ROWS as usize {
        return Err(invalid("AV1 tile layout beyond Annex A"));
    }
    info.size_bytes = 4;
    if info.cols_log2 > 0 || info.rows_log2 > 0 {
        info.context_update_tile_id = r.bits(info.rows_log2 + info.cols_log2)?;
        info.size_bytes = r.bits(2)? as usize + 1;
        if info.context_update_tile_id as usize >= info.count() {
            return Err(invalid("AV1 context_update_tile_id beyond the tiles"));
        }
    }
    Ok(info)
}

/// What the decoder keeps of one reference slot (7.20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RefSlot {
    pub valid: bool,
    pub surface: u32,
    /// `RefUpscaledWidth` (= `RefFrameWidth` while superres is refused).
    pub upscaled_width: u32,
    pub frame_height: u32,
    pub render_width: u32,
    pub render_height: u32,
    pub order_hint: u32,
    pub ref_deltas: [i8; 8],
    pub mode_deltas: [i8; 2],
    pub feature_enabled: [[bool; 8]; 8],
    pub feature_data: [[i16; 8]; 8],
}

impl Default for RefSlot {
    fn default() -> Self {
        Self {
            valid: false,
            surface: super::vaapi_ffi::VA_INVALID_ID,
            upscaled_width: 0,
            frame_height: 0,
            render_width: 0,
            render_height: 0,
            order_hint: 0,
            ref_deltas: DEFAULT_REF_DELTAS,
            mode_deltas: [0; 2],
            feature_enabled: [[false; 8]; 8],
            feature_data: [[0; 8]; 8],
        }
    }
}

/// `loop_filter_ref_deltas` of `setup_past_independence` (7.20):
/// INTRA 1, LAST/LAST2/LAST3 0, GOLDEN −1, BWDREF 0, ALTREF2/ALTREF −1.
pub(super) const DEFAULT_REF_DELTAS: [i8; 8] = [1, 0, 0, 0, -1, 0, -1, -1];
