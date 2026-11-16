//! Slice headers of the VA-API H.264 decoder (BK4, B-38): H.264 (08/2021)
//! 7.3.3 slice header with 7.3.3.1 reference list modification, 7.3.3.2
//! weight table and 7.3.3.3 reference marking (applied in
//! `vaapi_h264_dpb.rs`). Frames only (the contract has no fields), I and P
//! slices only (hosts never send B pictures).
use std::io;

use super::h264_bits::{unescape, BitReader, NAL_IDR};
use super::vaapi_h264_syntax::{Pps, Sps};
use crate::screen::core::wire::invalid;

fn unsupported(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, what)
}

/// One `modification_of_pic_nums_idc` with its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListModification {
    Subtract(u32),
    Add(u32),
    LongTerm(u32),
}

/// One `memory_management_control_operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mmco {
    UnmarkShort(u32),
    UnmarkLong(u32),
    ShortToLong(u32, u32),
    MaxLongTermIdx(u32),
    UnmarkAll,
    CurrentToLong(u32),
}

/// Explicit weights of list 0 (7.3.3.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Weights {
    pub luma_denom: u32,
    pub chroma_denom: u32,
    pub luma: Vec<Option<(i32, i32)>>,
    pub chroma: Vec<Option<[(i32, i32); 2]>>,
}

/// The parsed slice header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SliceHeader {
    pub nal_ref_idc: u8,
    pub idr: bool,
    pub first_mb: u32,
    /// `slice_type % 5`: 0 P, 2 I.
    pub slice_type: u32,
    pub pps_id: u32,
    pub frame_num: u32,
    pub poc_lsb: u32,
    pub delta_poc_bottom: i32,
    pub num_ref_idx_l0_minus1: u32,
    pub modifications: Vec<ListModification>,
    pub weights: Option<Weights>,
    pub long_term_reference: bool,
    pub mmco: Option<Vec<Mmco>>,
    pub cabac_init_idc: u32,
    pub slice_qp_delta: i32,
    pub disable_deblocking_filter_idc: u32,
    pub alpha_offset_div2: i32,
    pub beta_offset_div2: i32,
    /// Bits from the start of the NAL unit (header byte included) to the
    /// slice data, as VA-API's `slice_data_bit_offset` counts them.
    pub header_bits: usize,
}

/// The PPS id of a slice NAL unit (to find its SPS and PPS first).
pub(super) fn slice_pps_id(unit: &[u8]) -> io::Result<u32> {
    let rbsp = unescape(unit.get(1..unit.len().min(33)).unwrap_or_default());
    let mut r = BitReader::new(&rbsp);
    r.ue()?;
    r.ue()?;
    r.ue()
}

/// Parses the slice header of `unit` (header byte included).
pub(super) fn parse_slice(unit: &[u8], sps: &Sps, pps: &Pps) -> io::Result<SliceHeader> {
    let header = *unit
        .first()
        .ok_or_else(|| invalid("empty H.264 NAL unit"))?;
    let idr = header & 0x1f == NAL_IDR;
    let nal_ref_idc = (header >> 5) & 3;
    let rbsp = unescape(&unit[1..unit.len().min(4097)]);
    let mut r = BitReader::new(&rbsp);
    let first_mb = r.ue()?;
    let slice_type = r.ue()? % 5;
    match slice_type {
        0 | 2 => {}
        1 => {
            return Err(unsupported(
                "H.264 B slices are outside the screen contract",
            ))
        }
        _ => {
            return Err(unsupported(
                "H.264 SP/SI slices are outside the screen contract",
            ))
        }
    }
    if idr && slice_type != 2 {
        return Err(invalid("H.264 IDR with a P slice"));
    }
    let pps_id = r.ue()?;
    let frame_num = r.bits(sps.log2_max_frame_num)?;
    if idr {
        r.ue()?; // idr_pic_id
    }
    let (mut poc_lsb, mut delta_poc_bottom) = (0, 0);
    if sps.poc_type == 0 {
        poc_lsb = r.bits(sps.log2_max_poc_lsb)?;
        if pps.bottom_field_pic_order_in_frame_present {
            delta_poc_bottom = r.se()?;
        }
    }
    if pps.redundant_pic_cnt_present && r.ue()? != 0 {
        return Err(unsupported(
            "H.264 redundant pictures are outside the screen contract",
        ));
    }
    let mut num_ref_idx_l0_minus1 = pps.num_ref_idx_l0_default_minus1;
    let mut modifications = Vec::new();
    let mut weights = None;
    if slice_type == 0 {
        if r.flag()? {
            num_ref_idx_l0_minus1 = r.ue()?;
            if num_ref_idx_l0_minus1 > 31 {
                return Err(invalid("H.264 reference count out of range"));
            }
        }
        if r.flag()? {
            loop {
                let modification = match r.ue()? {
                    0 => ListModification::Subtract(r.ue()?.saturating_add(1)),
                    1 => ListModification::Add(r.ue()?.saturating_add(1)),
                    2 => ListModification::LongTerm(r.ue()?),
                    3 => break,
                    _ => return Err(invalid("H.264 reference list modification out of range")),
                };
                if modifications.len() > 32 {
                    return Err(invalid("H.264 reference list modification too long"));
                }
                modifications.push(modification);
            }
        }
        if pps.weighted_pred {
            weights = Some(weight_table(&mut r, num_ref_idx_l0_minus1)?);
        }
    }
    let mut long_term_reference = false;
    let mut mmco = None;
    if nal_ref_idc != 0 {
        if idr {
            r.flag()?; // no_output_of_prior_pics_flag
            long_term_reference = r.flag()?;
        } else if r.flag()? {
            mmco = Some(memory_operations(&mut r)?);
        }
    }
    let cabac_init_idc = if pps.entropy_coding_mode && slice_type != 2 {
        r.ue()?
    } else {
        0
    };
    let slice_qp_delta = r.se()?;
    let (mut disable_deblocking_filter_idc, mut alpha_offset_div2, mut beta_offset_div2) =
        (0, 0, 0);
    if pps.deblocking_filter_control_present {
        disable_deblocking_filter_idc = r.ue()?;
        if disable_deblocking_filter_idc != 1 {
            alpha_offset_div2 = r.se()?;
            beta_offset_div2 = r.se()?;
        }
    }
    // The NAL header byte counts; emulation prevention bytes do not.
    let header_bits = 8 + r.position();
    Ok(SliceHeader {
        nal_ref_idc,
        idr,
        first_mb,
        slice_type,
        pps_id,
        frame_num,
        poc_lsb,
        delta_poc_bottom,
        num_ref_idx_l0_minus1,
        modifications,
        weights,
        long_term_reference,
        mmco,
        cabac_init_idc,
        slice_qp_delta,
        disable_deblocking_filter_idc,
        alpha_offset_div2,
        beta_offset_div2,
        header_bits,
    })
}

fn weight_table(r: &mut BitReader<'_>, count_minus1: u32) -> io::Result<Weights> {
    let mut weights = Weights {
        luma_denom: r.ue()?,
        chroma_denom: r.ue()?,
        ..Weights::default()
    };
    if weights.luma_denom > 7 || weights.chroma_denom > 7 {
        return Err(invalid("H.264 weight denominator out of range"));
    }
    for _ in 0..=count_minus1 {
        weights.luma.push(if r.flag()? {
            Some((r.se()?, r.se()?))
        } else {
            None
        });
        weights.chroma.push(if r.flag()? {
            Some([(r.se()?, r.se()?), (r.se()?, r.se()?)])
        } else {
            None
        });
    }
    Ok(weights)
}

fn memory_operations(r: &mut BitReader<'_>) -> io::Result<Vec<Mmco>> {
    let mut operations = Vec::new();
    loop {
        let operation = match r.ue()? {
            0 => return Ok(operations),
            1 => Mmco::UnmarkShort(r.ue()?.saturating_add(1)),
            2 => Mmco::UnmarkLong(r.ue()?),
            3 => Mmco::ShortToLong(r.ue()?.saturating_add(1), r.ue()?),
            4 => Mmco::MaxLongTermIdx(r.ue()?),
            5 => Mmco::UnmarkAll,
            6 => Mmco::CurrentToLong(r.ue()?),
            _ => return Err(invalid("H.264 memory management operation out of range")),
        };
        if operations.len() > 66 {
            return Err(invalid("H.264 memory management list too long"));
        }
        operations.push(operation);
    }
}
