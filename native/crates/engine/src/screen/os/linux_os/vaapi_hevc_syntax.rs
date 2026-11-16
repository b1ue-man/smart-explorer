//! SPS, PPS and short-term reference picture sets of an HEVC stream for
//! the VA-API decoder (plan 2026-11-14 „Bildkette“ BK4b, B-38): ITU-T
//! H.265 (08/2021) 7.3.2.2 (SPS up to `strong_intra_smoothing_enabled_flag`,
//! the rest is not needed by VA-API), 7.3.2.3 (PPS), 7.3.3
//! (profile_tier_level), 7.3.7 with the derivation of 7.4.8 (RPS, also
//! inter-predicted). The screen contract is Main (or a Main-compatible
//! 8-bit stream), 4:2:0, progressive; refused with
//! `ErrorKind::Unsupported`, so the next decoder of the register opens:
//! other profiles and bit depths, separate colour planes, scaling lists,
//! range and screen content extensions (assumption A-HEVC-TOOLS: none of
//! them is written by NVENC, Media Foundation, AMF, MediaCodec or this
//! program's VA-API encoder with the settings of the project).
use std::io;

use super::h264_bits::{unescape, BitReader};
use crate::screen::core::wire::invalid;

pub(super) fn unsupported(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, what)
}

/// The RBSP of an HEVC NAL unit (two header bytes dropped).
pub(super) fn rbsp(unit: &[u8]) -> Vec<u8> {
    unescape(unit.get(2..).unwrap_or_default())
}

/// `nal_unit_type` and `TemporalId` of an HEVC NAL unit (7.3.1.2).
pub(super) fn nal_header(unit: &[u8]) -> Option<(u8, u8)> {
    let (first, second) = (*unit.first()?, *unit.get(1)?);
    let layer = (u16::from(first & 1) << 5) | u16::from(second >> 3);
    let temporal = (second & 7).checked_sub(1)?;
    (layer == 0).then_some(((first >> 1) & 0x3f, temporal))
}

/// One short-term reference picture set: deltas of POC with their
/// `used_by_curr_pic` flag, negative (S0, decreasing) and positive (S1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ShortTermRps {
    pub negative: Vec<(i32, bool)>,
    pub positive: Vec<(i32, bool)>,
}

impl ShortTermRps {
    pub(super) fn count(&self) -> usize {
        self.negative.len() + self.positive.len()
    }
}

/// `st_ref_pic_set(index)` (7.3.7, 7.4.8) given the sets before it (the
/// SPS's); `in_slice` for the set of a slice header (`index` is then
/// `num_short_term_ref_pic_sets` and `delta_idx_minus1` is coded).
pub(super) fn parse_st_rps(
    r: &mut BitReader<'_>,
    index: usize,
    sets: &[ShortTermRps],
    in_slice: bool,
) -> io::Result<ShortTermRps> {
    if index != 0 && r.flag()? {
        let delta_idx = if in_slice { r.ue()? as usize + 1 } else { 1 };
        let reference = index
            .checked_sub(delta_idx)
            .and_then(|at| sets.get(at))
            .ok_or_else(|| invalid("HEVC RPS predicts from a missing set"))?;
        let sign = r.flag()?;
        let magnitude = i32::try_from(r.ue()?)
            .ok()
            .and_then(|value| value.checked_add(1))
            .filter(|value| *value <= 1 << 15)
            .ok_or_else(|| invalid("HEVC RPS delta out of range"))?;
        let delta_rps = if sign { -magnitude } else { magnitude };
        let n = reference.count();
        let mut used = Vec::with_capacity(n + 1);
        let mut use_delta = Vec::with_capacity(n + 1);
        for _ in 0..=n {
            let flag = r.flag()?;
            used.push(flag);
            use_delta.push(flag || r.flag()?);
        }
        let negatives = reference.negative.len();
        let mut out = ShortTermRps::default();
        for (j, (delta, _)) in reference.positive.iter().enumerate().rev() {
            let poc = delta + delta_rps;
            if poc < 0 && use_delta[negatives + j] {
                out.negative.push((poc, used[negatives + j]));
            }
        }
        if delta_rps < 0 && use_delta[n] {
            out.negative.push((delta_rps, used[n]));
        }
        for (j, (delta, _)) in reference.negative.iter().enumerate() {
            let poc = delta + delta_rps;
            if poc < 0 && use_delta[j] {
                out.negative.push((poc, used[j]));
            }
        }
        for (j, (delta, _)) in reference.negative.iter().enumerate().rev() {
            let poc = delta + delta_rps;
            if poc > 0 && use_delta[j] {
                out.positive.push((poc, used[j]));
            }
        }
        if delta_rps > 0 && use_delta[n] {
            out.positive.push((delta_rps, used[n]));
        }
        for (j, (delta, _)) in reference.positive.iter().enumerate() {
            let poc = delta + delta_rps;
            if poc > 0 && use_delta[negatives + j] {
                out.positive.push((poc, used[negatives + j]));
            }
        }
        if out.count() > 16 {
            return Err(invalid("HEVC RPS larger than the DPB"));
        }
        return Ok(out);
    }
    let (negatives, positives) = (r.ue()?, r.ue()?);
    if negatives > 16 || positives > 16 || negatives + positives > 16 {
        return Err(invalid("HEVC RPS larger than the DPB"));
    }
    let mut out = ShortTermRps::default();
    let mut poc = 0i32;
    for _ in 0..negatives {
        poc -= delta_poc(r)?;
        out.negative.push((poc, r.flag()?));
    }
    poc = 0;
    for _ in 0..positives {
        poc += delta_poc(r)?;
        out.positive.push((poc, r.flag()?));
    }
    Ok(out)
}

/// `delta_poc_s0_minus1`/`delta_poc_s1_minus1` plus one; the values lie in
/// 0..=2^15 − 1 (7.4.8), so 16 of them stay far inside `i32`.
fn delta_poc(r: &mut BitReader<'_>) -> io::Result<i32> {
    let value = r.ue()?;
    if value > 0x7fff {
        return Err(invalid("HEVC RPS delta out of range"));
    }
    Ok(value as i32 + 1)
}

/// The parts of an SPS the decoder uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Sps {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub log2_max_poc_lsb: u32,
    pub max_dec_pic_buffering: u32,
    pub num_reorder_pics: u32,
    pub log2_min_cb: u32,
    pub log2_diff_max_min_cb: u32,
    pub log2_min_tb: u32,
    pub log2_diff_max_min_tb: u32,
    pub depth_inter: u32,
    pub depth_intra: u32,
    pub amp: bool,
    pub sao: bool,
    /// `pcm_enabled_flag` with its bit depths minus 1, log2 minimum size
    /// minus 3, log2 size difference and loop filter flag.
    pub pcm: Option<(u32, u32, u32, u32, bool)>,
    pub st_rps: Vec<ShortTermRps>,
    pub long_term_present: bool,
    pub num_long_term_sps: u32,
    pub temporal_mvp: bool,
    pub strong_intra_smoothing: bool,
}

impl Sps {
    pub(super) fn ctb_log2(&self) -> u32 {
        self.log2_min_cb.saturating_add(self.log2_diff_max_min_cb)
    }

    /// `PicWidthInCtbsY`, `PicHeightInCtbsY`.
    pub(super) fn ctbs(&self) -> (u32, u32) {
        let side = 1 << self.ctb_log2();
        (self.width.div_ceil(side), self.height.div_ceil(side))
    }
}

/// `profile_tier_level(1, sub_layers)`; refuses non-Main streams.
fn profile_tier_level(r: &mut BitReader<'_>, sub_layers: u32) -> io::Result<()> {
    r.bits(2)?; // general_profile_space
    r.bits(1)?; // general_tier_flag
    let profile = r.bits(5)?;
    let compatible = r.bits(32)?;
    let main_compatible = compatible & (1 << 30) != 0;
    if !(profile == 1 || profile == 2 || main_compatible) {
        return Err(unsupported("HEVC profile other than Main"));
    }
    r.bits(4)?; // source and constraint flags
    r.bits(32)?;
    r.bits(12)?; // the 43 reserved bits and general_inbld_flag
    r.bits(8)?; // general_level_idc
    let mut present = Vec::new();
    for _ in 0..sub_layers {
        present.push((r.flag()?, r.flag()?));
    }
    if sub_layers > 0 {
        r.bits(2 * (8 - sub_layers))?;
    }
    for (profile_present, level_present) in present {
        if profile_present {
            r.bits(32)?;
            r.bits(32)?;
            r.bits(24)?;
        }
        if level_present {
            r.bits(8)?;
        }
    }
    Ok(())
}

/// Parses an SPS NAL unit (header bytes included).
pub(super) fn parse_sps(unit: &[u8]) -> io::Result<Sps> {
    if unit.len() > 4096 {
        return Err(invalid("HEVC SPS too large"));
    }
    let data = rbsp(unit);
    let mut r = BitReader::new(&data);
    r.bits(4)?; // sps_video_parameter_set_id
    let sub_layers = r.bits(3)?;
    if sub_layers > 6 {
        return Err(invalid("HEVC SPS with more than 7 sub-layers"));
    }
    r.bits(1)?; // sps_temporal_id_nesting_flag
    profile_tier_level(&mut r, sub_layers)?;
    let mut sps = Sps {
        id: r.ue()?,
        ..Sps::default()
    };
    if sps.id > 15 {
        return Err(invalid("HEVC SPS id out of range"));
    }
    if r.ue()? != 1 {
        return Err(unsupported("HEVC chroma format is not 4:2:0"));
    }
    sps.width = r.ue()?;
    sps.height = r.ue()?;
    if !(1..=16_888).contains(&sps.width) || !(1..=16_888).contains(&sps.height) {
        return Err(invalid("HEVC SPS dimensions out of range"));
    }
    if r.flag()? {
        for _ in 0..4 {
            r.ue()?; // conf_win_*_offset (checked by the core)
        }
    }
    if r.ue()? != 0 || r.ue()? != 0 {
        return Err(unsupported(
            "HEVC above 8 bits is outside the screen contract",
        ));
    }
    sps.log2_max_poc_lsb = r.ue()?.saturating_add(4);
    if sps.log2_max_poc_lsb > 16 {
        return Err(invalid("HEVC log2_max_pic_order_cnt_lsb out of range"));
    }
    let ordering_all = r.flag()?;
    for _ in (if ordering_all { 0 } else { sub_layers })..=sub_layers {
        sps.max_dec_pic_buffering = r.ue()?.saturating_add(1);
        sps.num_reorder_pics = r.ue()?;
        r.ue()?; // sps_max_latency_increase_plus1
    }
    if sps.max_dec_pic_buffering > 16 {
        return Err(invalid("HEVC DPB larger than 16 pictures"));
    }
    sps.log2_min_cb = r.ue()?.saturating_add(3);
    sps.log2_diff_max_min_cb = r.ue()?;
    sps.log2_min_tb = r.ue()?.saturating_add(2);
    sps.log2_diff_max_min_tb = r.ue()?;
    if sps.ctb_log2() > 6 || sps.log2_min_tb.saturating_add(sps.log2_diff_max_min_tb) > 5 {
        return Err(invalid("HEVC block sizes out of range"));
    }
    sps.depth_inter = r.ue()?;
    sps.depth_intra = r.ue()?;
    if r.flag()? {
        return Err(unsupported("HEVC scaling lists are not decoded here"));
    }
    sps.amp = r.flag()?;
    sps.sao = r.flag()?;
    if r.flag()? {
        let depths = (r.bits(4)?, r.bits(4)?);
        let sizes = (r.ue()?, r.ue()?);
        sps.pcm = Some((depths.0, depths.1, sizes.0, sizes.1, r.flag()?));
    }
    let sets = r.ue()? as usize;
    if sets > 64 {
        return Err(invalid("HEVC SPS with more than 64 RPS"));
    }
    for index in 0..sets {
        let set = parse_st_rps(&mut r, index, &sps.st_rps, false)?;
        sps.st_rps.push(set);
    }
    sps.long_term_present = r.flag()?;
    if sps.long_term_present {
        sps.num_long_term_sps = r.ue()?;
        if sps.num_long_term_sps > 32 {
            return Err(invalid("HEVC SPS long-term list out of range"));
        }
        for _ in 0..sps.num_long_term_sps {
            r.bits(sps.log2_max_poc_lsb)?; // lt_ref_pic_poc_lsb_sps
            r.bits(1)?; // used_by_curr_pic_lt_sps_flag
        }
    }
    sps.temporal_mvp = r.flag()?;
    sps.strong_intra_smoothing = r.flag()?;
    Ok(sps)
}

/// The parts of a PPS the decoder uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Pps {
    pub id: u32,
    pub sps_id: u32,
    pub dependent_slices: bool,
    pub output_flag_present: bool,
    pub extra_slice_header_bits: u32,
    pub sign_data_hiding: bool,
    pub cabac_init_present: bool,
    pub num_ref_idx_l0_default: u32,
    pub num_ref_idx_l1_default: u32,
    pub init_qp_minus26: i32,
    pub constrained_intra_pred: bool,
    pub transform_skip: bool,
    pub cu_qp_delta: Option<u32>,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub slice_chroma_qp_offsets_present: bool,
    pub transquant_bypass: bool,
    /// Columns and rows (minus 1 each) of explicit tiles; `uniform` tiles
    /// get their sizes from the SPS.
    pub tiles: Option<(Vec<u32>, Vec<u32>, bool, bool)>,
    pub entropy_coding_sync: bool,
    pub loop_filter_across_slices: bool,
    pub deblocking_override_enabled: bool,
    pub deblocking_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    pub lists_modification_present: bool,
    pub log2_parallel_merge_level: u32,
    pub slice_header_extension_present: bool,
}

/// Parses a PPS NAL unit (header bytes included).
pub(super) fn parse_pps(unit: &[u8]) -> io::Result<Pps> {
    if unit.len() > 4096 {
        return Err(invalid("HEVC PPS too large"));
    }
    let data = rbsp(unit);
    let mut r = BitReader::new(&data);
    let mut pps = Pps {
        id: r.ue()?,
        sps_id: r.ue()?,
        ..Pps::default()
    };
    if pps.id > 63 || pps.sps_id > 15 {
        return Err(invalid("HEVC PPS id out of range"));
    }
    pps.dependent_slices = r.flag()?;
    pps.output_flag_present = r.flag()?;
    pps.extra_slice_header_bits = r.bits(3)?;
    pps.sign_data_hiding = r.flag()?;
    pps.cabac_init_present = r.flag()?;
    pps.num_ref_idx_l0_default = r.ue()?.saturating_add(1);
    pps.num_ref_idx_l1_default = r.ue()?.saturating_add(1);
    if pps.num_ref_idx_l0_default > 15 || pps.num_ref_idx_l1_default > 15 {
        return Err(invalid("HEVC reference count out of range"));
    }
    pps.init_qp_minus26 = r.se()?;
    pps.constrained_intra_pred = r.flag()?;
    pps.transform_skip = r.flag()?;
    if r.flag()? {
        pps.cu_qp_delta = Some(r.ue()?);
    }
    pps.cb_qp_offset = r.se()?;
    pps.cr_qp_offset = r.se()?;
    pps.slice_chroma_qp_offsets_present = r.flag()?;
    if r.flag()? || r.flag()? {
        return Err(unsupported("HEVC weighted prediction is not decoded here"));
    }
    pps.transquant_bypass = r.flag()?;
    let tiles = r.flag()?;
    pps.entropy_coding_sync = r.flag()?;
    if tiles {
        let (columns, rows) = (r.ue()?.saturating_add(1), r.ue()?.saturating_add(1));
        if columns > 20 || rows > 22 {
            return Err(invalid("HEVC tile count out of range"));
        }
        let uniform = r.flag()?;
        let (mut widths, mut heights) = (Vec::new(), Vec::new());
        if !uniform {
            for _ in 1..columns {
                widths.push(r.ue()?);
            }
            for _ in 1..rows {
                heights.push(r.ue()?);
            }
        }
        widths.resize(columns as usize - 1, 0);
        heights.resize(rows as usize - 1, 0);
        pps.tiles = Some((widths, heights, uniform, r.flag()?));
    }
    pps.loop_filter_across_slices = r.flag()?;
    if r.flag()? {
        pps.deblocking_override_enabled = r.flag()?;
        pps.deblocking_disabled = r.flag()?;
        if !pps.deblocking_disabled {
            pps.beta_offset_div2 = r.se()?;
            pps.tc_offset_div2 = r.se()?;
        }
    }
    if r.flag()? {
        return Err(unsupported("HEVC scaling lists are not decoded here"));
    }
    pps.lists_modification_present = r.flag()?;
    pps.log2_parallel_merge_level = r.ue()?.saturating_add(2);
    pps.slice_header_extension_present = r.flag()?;
    if r.flag()? {
        // pps_range/multilayer/3d/scc extension flags and 4 bits.
        if r.bits(4)? != 0 {
            return Err(unsupported("HEVC PPS extensions are not decoded here"));
        }
    }
    Ok(pps)
}
