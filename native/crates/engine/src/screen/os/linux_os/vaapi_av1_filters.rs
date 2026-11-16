//! The in-loop filter parameters of an AV1 frame header for the VA-API AV1
//! decoder (plan 2026-11-14 „Bildkette“ BK4b): `loop_filter_params()`
//! (5.9.11), `cdef_params()` (5.9.19) and `lr_params()` (5.9.20) of AV1
//! v1.0.0 with Errata 1, read into `FrameHeader` in the form
//! `VADecPictureParameterBufferAV1` of libva 2.20.0 wants them (CDEF
//! strengths as `pri << 2 | sec` with the coded `sec`, restoration types
//! after `Remap_Lr_Type`, the final `lr_unit_shift`).
use std::io;

use super::h264_bits::BitReader;
use super::vaapi_av1_bits::su;
use super::vaapi_av1_frame::FrameHeader;
use super::vaapi_av1_syntax::{SequenceHeader, DEFAULT_REF_DELTAS};

/// `Remap_Lr_Type` (5.9.20): NONE, SWITCHABLE, WIENER, SGRPROJ.
const REMAP_LR_TYPE: [u8; 4] = [0, 3, 1, 2];

/// `loop_filter_params()`; the deltas start from the primary reference
/// (already in `h`) or the defaults.
pub(super) fn loop_filter(r: &mut BitReader<'_>, h: &mut FrameHeader) -> io::Result<()> {
    if h.coded_lossless || h.allow_intrabc {
        h.loop_filter_level = [0; 4];
        h.ref_deltas = DEFAULT_REF_DELTAS;
        h.mode_deltas = [0; 2];
        return Ok(());
    }
    h.loop_filter_level[0] = r.bits(6)? as u8;
    h.loop_filter_level[1] = r.bits(6)? as u8;
    if h.loop_filter_level[0] != 0 || h.loop_filter_level[1] != 0 {
        h.loop_filter_level[2] = r.bits(6)? as u8;
        h.loop_filter_level[3] = r.bits(6)? as u8;
    }
    h.sharpness = r.bits(3)? as u8;
    h.delta_enabled = r.flag()?;
    if h.delta_enabled {
        h.delta_update = r.flag()?;
        if h.delta_update {
            for index in 0..8 {
                if r.flag()? {
                    h.ref_deltas[index] = su(r, 7)? as i8;
                }
            }
            for index in 0..2 {
                if r.flag()? {
                    h.mode_deltas[index] = su(r, 7)? as i8;
                }
            }
        }
    }
    Ok(())
}

/// `cdef_params()`.
pub(super) fn cdef(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    h: &mut FrameHeader,
) -> io::Result<()> {
    if h.coded_lossless || h.allow_intrabc || !seq.enable_cdef {
        h.cdef_bits = 0;
        h.cdef_y = [0; 8];
        h.cdef_uv = [0; 8];
        h.cdef_damping_minus_3 = 0;
        return Ok(());
    }
    h.cdef_damping_minus_3 = r.bits(2)? as u8;
    h.cdef_bits = r.bits(2)? as u8;
    for index in 0..(1usize << h.cdef_bits) {
        let (pri, sec) = (r.bits(4)?, r.bits(2)?);
        h.cdef_y[index] = ((pri << 2) | sec) as u8;
        let (pri, sec) = (r.bits(4)?, r.bits(2)?);
        h.cdef_uv[index] = ((pri << 2) | sec) as u8;
    }
    Ok(())
}

/// `lr_params()`.
pub(super) fn restoration(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    h: &mut FrameHeader,
) -> io::Result<()> {
    // AllLossless = CodedLossless while superres is refused.
    if h.coded_lossless || h.allow_intrabc || !seq.enable_restoration {
        h.lr_type = [0; 3];
        return Ok(());
    }
    let (mut uses, mut chroma) = (false, false);
    for plane in 0..3 {
        let kind = REMAP_LR_TYPE[r.bits(2)? as usize];
        h.lr_type[plane] = kind;
        if kind != 0 {
            uses = true;
            chroma |= plane > 0;
        }
    }
    if uses {
        let mut shift = r.bits(1)? as u8;
        if seq.use_128x128_superblock {
            shift += 1;
        } else if shift == 1 {
            shift += r.bits(1)? as u8;
        }
        h.lr_unit_shift = shift;
        // Profile 0 is 4:2:0 (subsampling_x and _y).
        if chroma {
            h.lr_uv_shift = r.bits(1)? as u8;
        }
    }
    Ok(())
}
