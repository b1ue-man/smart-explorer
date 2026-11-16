//! SPS and PPS of an H.264 stream for the VA-API decoder (BK4, B-38):
//! VA-API decodes slices only, the program parses the parameter sets
//! (H.264 (08/2021) 7.3.2.1.1, 7.3.2.2, scaling lists 7.3.2.1.1.1 with the
//! fall-back rules of Table 7-2). The screen contract is progressive 8-bit
//! 4:2:0 without B pictures; anything else is refused with an error, which
//! makes the decoder give way to the next one (`vaapi_decode.rs`).
use std::io;

use super::h264_bits::{unescape, BitReader};
use crate::screen::core::wire::invalid;

/// Default scaling lists (Table 7-3, 7-4) in zig-zag order.
const DEFAULT_4X4_INTRA: [u8; 16] = [
    6, 13, 13, 20, 20, 20, 28, 28, 28, 28, 32, 32, 32, 37, 37, 42,
];
const DEFAULT_4X4_INTER: [u8; 16] = [
    10, 14, 14, 20, 20, 20, 24, 24, 24, 24, 27, 27, 27, 30, 30, 34,
];
const DEFAULT_8X8_INTRA: [u8; 64] = [
    6, 10, 10, 13, 11, 13, 16, 16, 16, 16, 18, 18, 18, 18, 18, 23, 23, 23, 23, 23, 23, 25, 25, 25,
    25, 25, 25, 25, 27, 27, 27, 27, 27, 27, 27, 27, 29, 29, 29, 29, 29, 29, 29, 31, 31, 31, 31, 31,
    31, 33, 33, 33, 33, 33, 36, 36, 36, 36, 38, 38, 38, 40, 40, 42,
];
const DEFAULT_8X8_INTER: [u8; 64] = [
    9, 13, 13, 15, 13, 15, 17, 17, 17, 17, 19, 19, 19, 19, 19, 21, 21, 21, 21, 21, 21, 22, 22, 22,
    22, 22, 22, 22, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27, 27,
    27, 28, 28, 28, 28, 28, 30, 30, 30, 30, 32, 32, 32, 33, 33, 35,
];

/// The six 4×4 and two 8×8 scaling lists (zig-zag order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScalingLists {
    pub list4x4: [[u8; 16]; 6],
    pub list8x8: [[u8; 64]; 2],
}

impl ScalingLists {
    pub(super) fn flat() -> Self {
        Self {
            list4x4: [[16; 16]; 6],
            list8x8: [[16; 64]; 2],
        }
    }
}

/// The parts of a sequence parameter set the decoder uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sps {
    pub id: u32,
    pub chroma_format_idc: u32,
    pub scaling: Option<ScalingLists>,
    pub log2_max_frame_num: u32,
    pub poc_type: u32,
    pub log2_max_poc_lsb: u32,
    pub delta_pic_order_always_zero: bool,
    pub max_num_ref_frames: u32,
    pub gaps_allowed: bool,
    pub width_mbs: u32,
    pub height_mbs: u32,
    pub direct_8x8_inference: bool,
}

impl Sps {
    pub(super) fn max_frame_num(&self) -> u32 {
        1 << self.log2_max_frame_num
    }
}

/// The parts of a picture parameter set the decoder uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pps {
    pub id: u32,
    pub sps_id: u32,
    pub entropy_coding_mode: bool,
    pub bottom_field_pic_order_in_frame_present: bool,
    pub num_ref_idx_l0_default_minus1: u32,
    pub weighted_pred: bool,
    pub weighted_bipred_idc: u32,
    pub pic_init_qp_minus26: i32,
    pub pic_init_qs_minus26: i32,
    pub chroma_qp_index_offset: i32,
    pub deblocking_filter_control_present: bool,
    pub constrained_intra_pred: bool,
    pub redundant_pic_cnt_present: bool,
    pub transform_8x8_mode: bool,
    /// Explicit lists of the PPS (already with the SPS fall-back applied).
    pub scaling: Option<ScalingLists>,
    pub second_chroma_qp_index_offset: i32,
}

fn unsupported(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, what)
}

/// One `scaling_list()` into `out`; `None` keeps the fall-back, `Some(true)`
/// selects the default list.
fn scaling_list(reader: &mut BitReader<'_>, out: &mut [u8]) -> io::Result<bool> {
    let (mut last, mut next) = (8i32, 8i32);
    for (index, slot) in out.iter_mut().enumerate() {
        if next != 0 {
            let delta = reader.se()?;
            if !(-128..=127).contains(&delta) {
                return Err(invalid("H.264 scaling list delta out of range"));
            }
            next = (last + delta + 256) % 256;
            if index == 0 && next == 0 {
                return Ok(true);
            }
        }
        let value = if next == 0 { last } else { next };
        *slot = value as u8;
        last = value;
    }
    Ok(false)
}

/// Lists of an SPS or PPS (`count` = 6 or 8 entries); `fallback` gives the
/// rule A (SPS: defaults) or rule B (PPS: the SPS lists) base.
// Each list falls back to the one before it, so the index is needed.
#[allow(clippy::needless_range_loop)]
fn scaling_lists(
    reader: &mut BitReader<'_>,
    count: usize,
    fallback: &ScalingLists,
) -> io::Result<ScalingLists> {
    let mut lists = ScalingLists::flat();
    for index in 0..count {
        let present = reader.flag()?;
        if index < 6 {
            let fall = match index {
                0 => fallback.list4x4[0],
                3 => fallback.list4x4[3],
                _ => lists.list4x4[index - 1],
            };
            let default = if index < 3 {
                DEFAULT_4X4_INTRA
            } else {
                DEFAULT_4X4_INTER
            };
            lists.list4x4[index] = if !present {
                fall
            } else if scaling_list(reader, &mut lists.list4x4[index])? {
                default
            } else {
                lists.list4x4[index]
            };
        } else {
            let slot = index - 6;
            let fall = fallback.list8x8[slot];
            let default = if slot == 0 {
                DEFAULT_8X8_INTRA
            } else {
                DEFAULT_8X8_INTER
            };
            lists.list8x8[slot] = if !present {
                fall
            } else if scaling_list(reader, &mut lists.list8x8[slot])? {
                default
            } else {
                lists.list8x8[slot]
            };
        }
    }
    if count < 8 {
        lists.list8x8 = fallback.list8x8;
    }
    Ok(lists)
}

/// The defaults that rule A falls back to.
fn default_lists() -> ScalingLists {
    ScalingLists {
        list4x4: [
            DEFAULT_4X4_INTRA,
            DEFAULT_4X4_INTRA,
            DEFAULT_4X4_INTRA,
            DEFAULT_4X4_INTER,
            DEFAULT_4X4_INTER,
            DEFAULT_4X4_INTER,
        ],
        list8x8: [DEFAULT_8X8_INTRA, DEFAULT_8X8_INTER],
    }
}

/// Parses an SPS NAL unit (header byte included).
pub(super) fn parse_sps(unit: &[u8]) -> io::Result<Sps> {
    if unit.len() > 4096 {
        return Err(invalid("H.264 SPS too large"));
    }
    let rbsp = unescape(unit.get(1..).unwrap_or_default());
    let mut r = BitReader::new(&rbsp);
    let profile_idc = r.bits(8)? as u8;
    r.bits(16)?; // constraint flags, reserved bits, level_idc
    let id = r.ue()?;
    if id > 31 {
        return Err(invalid("H.264 SPS id out of range"));
    }
    let mut chroma_format_idc = 1;
    let mut scaling = None;
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format_idc = r.ue()?;
        if chroma_format_idc == 3 && r.flag()? {
            return Err(unsupported(
                "H.264 separate colour planes are outside the screen contract",
            ));
        }
        if r.ue()? != 0 || r.ue()? != 0 {
            return Err(unsupported(
                "H.264 above 8 bits is outside the screen contract",
            ));
        }
        r.flag()?; // qpprime_y_zero_transform_bypass_flag
        if r.flag()? {
            let count = if chroma_format_idc == 3 { 12 } else { 8 };
            if count != 8 {
                return Err(unsupported(
                    "H.264 4:4:4 scaling lists are outside the screen contract",
                ));
            }
            scaling = Some(scaling_lists(&mut r, count, &default_lists())?);
        }
    }
    if chroma_format_idc != 1 {
        return Err(unsupported("H.264 chroma format is not 4:2:0"));
    }
    let log2_max_frame_num = r.ue()?.saturating_add(4);
    if log2_max_frame_num > 16 {
        return Err(invalid("H.264 log2_max_frame_num out of range"));
    }
    let poc_type = r.ue()?;
    let mut log2_max_poc_lsb = 0;
    let delta_pic_order_always_zero = false;
    match poc_type {
        0 => {
            log2_max_poc_lsb = r.ue()?.saturating_add(4);
            if log2_max_poc_lsb > 16 {
                return Err(invalid("H.264 log2_max_pic_order_cnt_lsb out of range"));
            }
        }
        2 => {}
        _ => {
            return Err(unsupported(
                "H.264 picture order count type 1 is not decoded here",
            ))
        }
    }
    let max_num_ref_frames = r.ue()?;
    if max_num_ref_frames > 16 {
        return Err(invalid("H.264 max_num_ref_frames out of range"));
    }
    let gaps_allowed = r.flag()?;
    let width_mbs = r.ue()?.saturating_add(1);
    let height_map_units = r.ue()?.saturating_add(1);
    if !r.flag()? {
        return Err(unsupported(
            "H.264 field coding is outside the screen contract",
        ));
    }
    let direct_8x8_inference = r.flag()?;
    if width_mbs > 512 || height_map_units > 512 {
        return Err(invalid("H.264 picture larger than the decoder accepts"));
    }
    Ok(Sps {
        id,
        chroma_format_idc,
        scaling,
        log2_max_frame_num,
        poc_type,
        log2_max_poc_lsb,
        delta_pic_order_always_zero,
        max_num_ref_frames,
        gaps_allowed,
        width_mbs,
        height_mbs: height_map_units,
        direct_8x8_inference,
    })
}

/// Parses a PPS NAL unit (header byte included) whose SPS is `sps_of(id)`.
pub(super) fn parse_pps(unit: &[u8], sps_of: impl Fn(u32) -> Option<Sps>) -> io::Result<Pps> {
    if unit.len() > 4096 {
        return Err(invalid("H.264 PPS too large"));
    }
    let rbsp = unescape(unit.get(1..).unwrap_or_default());
    let mut r = BitReader::new(&rbsp);
    let id = r.ue()?;
    let sps_id = r.ue()?;
    if id > 255 || sps_id > 31 {
        return Err(invalid("H.264 PPS id out of range"));
    }
    let sps = sps_of(sps_id).ok_or_else(|| invalid("H.264 PPS names an unknown SPS"))?;
    let entropy_coding_mode = r.flag()?;
    let bottom_field_pic_order_in_frame_present = r.flag()?;
    if r.ue()? != 0 {
        return Err(unsupported(
            "H.264 slice groups are outside the screen contract",
        ));
    }
    let num_ref_idx_l0_default_minus1 = r.ue()?;
    let num_ref_idx_l1_default_minus1 = r.ue()?;
    if num_ref_idx_l0_default_minus1 > 31 || num_ref_idx_l1_default_minus1 > 31 {
        return Err(invalid("H.264 reference count out of range"));
    }
    let weighted_pred = r.flag()?;
    let weighted_bipred_idc = r.bits(2)?;
    let pic_init_qp_minus26 = r.se()?;
    let pic_init_qs_minus26 = r.se()?;
    let chroma_qp_index_offset = r.se()?;
    let deblocking_filter_control_present = r.flag()?;
    let constrained_intra_pred = r.flag()?;
    let redundant_pic_cnt_present = r.flag()?;
    let mut transform_8x8_mode = false;
    let mut scaling = None;
    let mut second_chroma_qp_index_offset = chroma_qp_index_offset;
    if r.more_rbsp_data() {
        transform_8x8_mode = r.flag()?;
        if r.flag()? {
            let count = 6 + if transform_8x8_mode { 2 } else { 0 };
            let base = sps.scaling.clone().unwrap_or_else(default_lists);
            scaling = Some(scaling_lists(&mut r, count, &base)?);
        }
        second_chroma_qp_index_offset = r.se()?;
    }
    if !(-12..=12).contains(&chroma_qp_index_offset)
        || !(-12..=12).contains(&second_chroma_qp_index_offset)
    {
        return Err(invalid("H.264 chroma QP offset out of range"));
    }
    Ok(Pps {
        id,
        sps_id,
        entropy_coding_mode,
        bottom_field_pic_order_in_frame_present,
        num_ref_idx_l0_default_minus1,
        weighted_pred,
        weighted_bipred_idc,
        pic_init_qp_minus26,
        pic_init_qs_minus26,
        chroma_qp_index_offset,
        deblocking_filter_control_present,
        constrained_intra_pred,
        redundant_pic_cnt_present,
        transform_8x8_mode,
        scaling,
        second_chroma_qp_index_offset,
    })
}

/// The lists the IQ matrix buffer gets for `sps` and `pps`: the PPS's own,
/// else the SPS's, else flat (no matrix in the stream).
pub(super) fn effective_lists(sps: &Sps, pps: &Pps) -> ScalingLists {
    pps.scaling
        .clone()
        .or_else(|| sps.scaling.clone())
        .unwrap_or_else(ScalingLists::flat)
}
