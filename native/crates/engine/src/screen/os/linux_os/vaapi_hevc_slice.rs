//! Slice segment headers of the VA-API HEVC decoder (plan 2026-11-14
//! „Bildkette“ BK4b, B-38): ITU-T H.265 (08/2021) 7.3.6.1 with the
//! reference picture list modification of 7.3.6.2; the prediction weight
//! table never occurs (weighted prediction is refused at the PPS). Long-term
//! reference pictures are parsed and refused when a slice uses one
//! (`ErrorKind::Unsupported`; assumption A-HEVC-TOOLS). A dependent slice
//! segment takes the fields of the independent one before it (7.4.7.1).
use std::collections::BTreeMap;
use std::io;

use super::h264_bits::BitReader;
use super::vaapi_hevc_syntax::{parse_st_rps, rbsp, unsupported, Pps, ShortTermRps, Sps};
use crate::screen::core::wire::invalid;

/// `slice_type` (Table 7-7).
pub(super) const SLICE_B: u8 = 0;
pub(super) const SLICE_P: u8 = 1;
pub(super) const SLICE_I: u8 = 2;
/// IDR NAL unit types (Table 7-1) and the IRAP range.
pub(super) const NAL_IDR_W_RADL: u8 = 19;
pub(super) const NAL_IDR_N_LP: u8 = 20;
pub(super) const NAL_CRA: u8 = 21;

/// `Ceil(Log2(n))`.
pub(super) fn ceil_log2(n: u32) -> u32 {
    if n <= 1 {
        0
    } else {
        32 - (n - 1).leading_zeros()
    }
}

/// The parsed slice segment header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SliceHeader {
    pub nal_type: u8,
    pub first: bool,
    pub pps_id: u32,
    pub dependent: bool,
    pub address: u32,
    pub slice_type: u8,
    pub poc_lsb: u32,
    /// The short-term RPS of the picture (empty for an IDR).
    pub rps: ShortTermRps,
    /// Bits of an RPS coded in the slice (VA-API `st_rps_bits`), else 0.
    pub st_rps_bits: u32,
    pub temporal_mvp: bool,
    pub sao_luma: bool,
    pub sao_chroma: bool,
    /// Active references of list 0 and 1.
    pub num_ref_idx: [u32; 2],
    pub list_entry: [Option<Vec<u32>>; 2],
    pub mvd_l1_zero: bool,
    pub cabac_init: bool,
    pub collocated_from_l0: bool,
    pub collocated_ref_idx: u32,
    pub five_minus_max_merge_cand: u32,
    pub qp_delta: i32,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub deblocking_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    pub loop_filter_across_slices: bool,
    pub entry_points: u32,
    pub entry_point_offsets: Vec<u32>,
    /// Bytes from the NAL header to `slice_data()` without emulation
    /// prevention bytes (VA-API `slice_data_byte_offset`).
    pub header_bytes: usize,
}

impl SliceHeader {
    pub(super) fn idr(&self) -> bool {
        matches!(self.nal_type, NAL_IDR_W_RADL | NAL_IDR_N_LP)
    }

    pub(super) fn irap(&self) -> bool {
        (16..=23).contains(&self.nal_type)
    }

    /// `NumPicTotalCurr` (7-55) without long-term pictures.
    pub(super) fn total_curr(&self) -> u32 {
        let used = |list: &[(i32, bool)]| list.iter().filter(|(_, used)| *used).count();
        (used(&self.rps.negative) + used(&self.rps.positive)) as u32
    }
}

/// Parses the slice segment header of `unit` (header bytes included) of
/// NAL type `nal_type`; `previous` is the independent slice segment of the
/// same picture.
pub(super) fn parse_slice(
    unit: &[u8],
    nal_type: u8,
    sps_map: &BTreeMap<u32, Sps>,
    pps_map: &BTreeMap<u32, Pps>,
    previous: Option<&SliceHeader>,
) -> io::Result<SliceHeader> {
    let data = rbsp(unit.get(..unit.len().min(4098)).unwrap_or(unit));
    let mut r = BitReader::new(&data);
    let first = r.flag()?;
    if (16..=23).contains(&nal_type) {
        r.bits(1)?; // no_output_of_prior_pics_flag
    }
    let pps_id = r.ue()?;
    let pps = pps_map
        .get(&pps_id)
        .ok_or_else(|| invalid("HEVC slice names an unknown PPS"))?;
    let sps = sps_map
        .get(&pps.sps_id)
        .ok_or_else(|| invalid("HEVC PPS names an unknown SPS"))?;
    let (columns, rows) = sps.ctbs();
    let total_ctbs = columns.saturating_mul(rows);
    let (dependent, address) = if first {
        (false, 0)
    } else {
        let dependent = pps.dependent_slices && r.flag()?;
        let address = r.bits(ceil_log2(total_ctbs))?;
        if address >= total_ctbs {
            return Err(invalid("HEVC slice address beyond the picture"));
        }
        (dependent, address)
    };
    let mut h = if dependent {
        previous
            .cloned()
            .ok_or_else(|| invalid("HEVC dependent slice without its slice"))?
    } else {
        independent(&mut r, nal_type, sps, pps)?
    };
    h.nal_type = nal_type;
    h.first = first;
    h.pps_id = pps_id;
    h.dependent = dependent;
    h.address = address;
    h.entry_points = 0;
    h.entry_point_offsets.clear();
    if pps.tiles.is_some() || pps.entropy_coding_sync {
        h.entry_points = r.ue()?;
        if h.entry_points > total_ctbs {
            return Err(invalid("HEVC entry points beyond the picture"));
        }
        if h.entry_points > 0 {
            let length = r.ue()?.saturating_add(1);
            if length > 32 {
                return Err(invalid("HEVC entry point offset too long"));
            }
            h.entry_point_offsets.reserve(h.entry_points as usize);
            for _ in 0..h.entry_points {
                h.entry_point_offsets
                    .push(r.bits(length)?.saturating_add(1));
            }
        }
    }
    if pps.slice_header_extension_present {
        let length = r.ue()?;
        if length > 256 {
            return Err(invalid("HEVC slice header extension too long"));
        }
        for _ in 0..length {
            r.bits(8)?;
        }
    }
    // byte_alignment(): a one bit, then zero bits to the byte boundary.
    if !r.flag()? {
        return Err(invalid("HEVC slice header without its alignment bit"));
    }
    while r.position() % 8 != 0 {
        r.bits(1)?;
    }
    h.header_bytes = 2 + r.position() / 8;
    Ok(h)
}

/// The fields of an independent slice segment (7.3.6.1).
fn independent(
    r: &mut BitReader<'_>,
    nal_type: u8,
    sps: &Sps,
    pps: &Pps,
) -> io::Result<SliceHeader> {
    let mut h = SliceHeader::default();
    for _ in 0..pps.extra_slice_header_bits {
        r.bits(1)?; // slice_reserved_flag
    }
    h.slice_type = match r.ue()? {
        0 => SLICE_B,
        1 => SLICE_P,
        2 => SLICE_I,
        _ => return Err(invalid("HEVC slice type out of range")),
    };
    if pps.output_flag_present {
        r.bits(1)?; // pic_output_flag
    }
    if !matches!(nal_type, NAL_IDR_W_RADL | NAL_IDR_N_LP) {
        h.poc_lsb = r.bits(sps.log2_max_poc_lsb)?;
        if !r.flag()? {
            let start = r.position();
            h.rps = parse_st_rps(r, sps.st_rps.len(), &sps.st_rps, true)?;
            h.st_rps_bits = (r.position() - start) as u32;
        } else {
            let index = if sps.st_rps.len() > 1 {
                r.bits(ceil_log2(sps.st_rps.len() as u32))? as usize
            } else {
                0
            };
            h.rps = sps
                .st_rps
                .get(index)
                .cloned()
                .ok_or_else(|| invalid("HEVC slice names a missing RPS"))?;
        }
        if sps.long_term_present {
            let from_sps = if sps.num_long_term_sps > 0 {
                r.ue()?
            } else {
                0
            };
            let from_slice = r.ue()?;
            if from_sps > 0 || from_slice > 0 {
                return Err(unsupported(
                    "HEVC long-term references are not decoded here",
                ));
            }
        }
        if sps.temporal_mvp {
            h.temporal_mvp = r.flag()?;
        }
    }
    if sps.sao {
        h.sao_luma = r.flag()?;
        h.sao_chroma = r.flag()?;
    }
    h.collocated_from_l0 = true;
    if h.slice_type != SLICE_I {
        h.num_ref_idx = [pps.num_ref_idx_l0_default, pps.num_ref_idx_l1_default];
        if r.flag()? {
            h.num_ref_idx[0] = r.ue()?.saturating_add(1);
            if h.slice_type == SLICE_B {
                h.num_ref_idx[1] = r.ue()?.saturating_add(1);
            }
        }
        if h.slice_type != SLICE_B {
            h.num_ref_idx[1] = 0;
        }
        if h.num_ref_idx.iter().any(|count| *count > 15) {
            return Err(invalid("HEVC reference count out of range"));
        }
        let total = h.total_curr();
        if total == 0 {
            return Err(invalid("HEVC inter slice without a current reference"));
        }
        if pps.lists_modification_present && total > 1 {
            let lists = if h.slice_type == SLICE_B { 2 } else { 1 };
            for list in 0..lists {
                if r.flag()? {
                    let entries = (0..h.num_ref_idx[list])
                        .map(|_| r.bits(ceil_log2(total)))
                        .collect::<io::Result<Vec<_>>>()?;
                    h.list_entry[list] = Some(entries);
                }
            }
        }
        if h.slice_type == SLICE_B {
            h.mvd_l1_zero = r.flag()?;
        }
        if pps.cabac_init_present {
            h.cabac_init = r.flag()?;
        }
        if h.temporal_mvp {
            if h.slice_type == SLICE_B {
                h.collocated_from_l0 = r.flag()?;
            }
            let list = if h.collocated_from_l0 { 0 } else { 1 };
            if h.num_ref_idx[list] > 1 {
                h.collocated_ref_idx = r.ue()?;
                if h.collocated_ref_idx >= h.num_ref_idx[list] {
                    return Err(invalid("HEVC collocated_ref_idx out of range"));
                }
            }
        }
        h.five_minus_max_merge_cand = r.ue()?;
        if h.five_minus_max_merge_cand > 4 {
            return Err(invalid("HEVC merge candidates out of range"));
        }
    }
    h.qp_delta = r.se()?;
    if pps.slice_chroma_qp_offsets_present {
        h.cb_qp_offset = r.se()?;
        h.cr_qp_offset = r.se()?;
    }
    h.deblocking_disabled = pps.deblocking_disabled;
    h.beta_offset_div2 = pps.beta_offset_div2;
    h.tc_offset_div2 = pps.tc_offset_div2;
    if pps.deblocking_override_enabled && r.flag()? {
        h.deblocking_disabled = r.flag()?;
        if !h.deblocking_disabled {
            h.beta_offset_div2 = r.se()?;
            h.tc_offset_div2 = r.se()?;
        }
    }
    h.loop_filter_across_slices = pps.loop_filter_across_slices;
    if pps.loop_filter_across_slices && (h.sao_luma || h.sao_chroma || !h.deblocking_disabled) {
        h.loop_filter_across_slices = r.flag()?;
    }
    Ok(h)
}
