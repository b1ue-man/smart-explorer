//! VA-API HEVC decoder of the Linux viewer (plan 2026-11-14 „Bildkette“
//! BK4b, B-38): the program parses SPS, PPS and slice segment headers and
//! keeps the reference pictures (`vaapi_hevc_syntax.rs`,
//! `vaapi_hevc_slice.rs`, `vaapi_hevc_dpb.rs`); the GPU decodes the slices
//! into NV12, which `vaapi_decode_core.rs` reads back into the plane path.
//! One access unit (Annex B, one picture) per record. VPS, SEI, access unit
//! delimiters and end-of-sequence units carry nothing VA-API needs and are
//! skipped; units of other layers (`nuh_layer_id` > 0) are skipped as a
//! single-layer decoder does (H.265 F.7.4.2.4.4).
use std::collections::BTreeMap;
use std::io;

use super::h264_bits::nal_units;
use super::vaapi_decode_core::{UnitDecoder, VaDecodeSession};
use super::vaapi_ffi::{
    VaId, VA_PICTURE_PARAMETER_BUFFER, VA_SLICE_DATA_BUFFER, VA_SLICE_PARAMETER_BUFFER,
    VA_SUBSETS_PARAMETER_BUFFER,
};
use super::vaapi_hevc_decode_params::{picture_params, slice_params};
use super::vaapi_hevc_dpb::{check_nal_type, Dpb};
use super::vaapi_hevc_slice::{parse_slice, SliceHeader};
use super::vaapi_hevc_syntax::{nal_header, parse_pps, parse_sps, Pps, Sps};
use super::vaapi_vpp::Buffers;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;

const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;
/// Surfaces besides the DPB: the current picture and one spare.
const EXTRA_SURFACES: usize = 2;

/// The surface size of a coded picture: whole 16 × 16 blocks (assumption
/// A-VA-HEVC-DEC-SURFACE).
fn surface_size(sps: &Sps) -> FrameSize {
    FrameSize::new(sps.width.div_ceil(16) * 16, sps.height.div_ceil(16) * 16)
}

/// The HEVC part of a VA-API decoder of one picture group.
pub(super) struct HevcUnits {
    size: FrameSize,
    sps: BTreeMap<u32, Sps>,
    pps: BTreeMap<u32, Pps>,
    dpb: Dpb,
}

impl HevcUnits {
    pub(super) fn new(size: FrameSize) -> Self {
        Self {
            size,
            sps: BTreeMap::new(),
            pps: BTreeMap::new(),
            dpb: Dpb::default(),
        }
    }

    /// The slice segment headers of one picture, checked to agree.
    fn headers(&self, slices: &[(&[u8], u8)]) -> io::Result<Vec<SliceHeader>> {
        let mut headers: Vec<SliceHeader> = Vec::with_capacity(slices.len());
        let mut independent: Option<SliceHeader> = None;
        for (index, (unit, nal_type)) in slices.iter().enumerate() {
            check_nal_type(*nal_type)?;
            let header = parse_slice(unit, *nal_type, &self.sps, &self.pps, independent.as_ref())?;
            if header.first != (index == 0) {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "HEVC record does not hold exactly one picture",
                ));
            }
            if let Some(first) = headers.first() {
                if header.pps_id != first.pps_id || header.nal_type != first.nal_type {
                    return Err(invalid("HEVC slices of one picture disagree"));
                }
            }
            if !header.dependent {
                independent = Some(header.clone());
            }
            headers.push(header);
        }
        Ok(headers)
    }
}

impl UnitDecoder for HevcUnits {
    fn decode_unit(
        &mut self,
        va: &mut VaDecodeSession,
        payload: &[u8],
    ) -> io::Result<Option<VaId>> {
        let mut slices: Vec<(&[u8], u8)> = Vec::new();
        let mut temporal_id = 0;
        for unit in nal_units(payload) {
            let Some((nal_type, tid)) = nal_header(unit) else {
                continue;
            };
            match nal_type {
                NAL_SPS => {
                    let sps = parse_sps(unit)?;
                    self.sps.insert(sps.id, sps);
                }
                NAL_PPS => {
                    let pps = parse_pps(unit)?;
                    self.pps.insert(pps.id, pps);
                }
                0..=31 => {
                    temporal_id = tid;
                    slices.push((unit, nal_type));
                }
                // VPS, AUD, EOS, EOB, filler, SEI and reserved units.
                _ => {}
            }
        }
        if slices.is_empty() || slices.len() > 4096 {
            return if slices.is_empty() {
                Ok(None)
            } else {
                Err(invalid("HEVC picture has too many slices"))
            };
        }
        let headers = self.headers(&slices)?;
        if !headers
            .windows(2)
            .all(|pair| pair[1].address > pair[0].address)
        {
            return Err(invalid("HEVC slice addresses not strictly increasing"));
        }
        let first = &headers[0];
        let pps = self
            .pps
            .get(&first.pps_id)
            .ok_or_else(|| invalid("HEVC slice names an unknown PPS"))?;
        let sps = self
            .sps
            .get(&pps.sps_id)
            .ok_or_else(|| invalid("HEVC PPS names an unknown SPS"))?;
        if sps.width < self.size.width || sps.height < self.size.height {
            return Err(invalid("HEVC SPS smaller than its picture group"));
        }
        let (columns, rows) = sps.ctbs();
        if slices.len() > columns.saturating_mul(rows) as usize {
            return Err(invalid("HEVC picture has more slices than CTBs"));
        }
        let coded = surface_size(sps);
        let count = sps.max_dec_pic_buffering as usize + EXTRA_SURFACES;
        if first.irap() {
            if va.context_for(coded, count)? {
                self.dpb = Dpb::default();
            }
        } else if va
            .context
            .as_ref()
            .is_none_or(|c| c.coded != coded || count > c.surfaces.len())
        {
            return Err(invalid("HEVC non-IRAP picture without matching VA context"));
        }
        let poc = self.dpb.poc(first, sps, temporal_id)?;
        let refs = self.dpb.apply_rps(first, poc)?;
        if refs.frames.len() >= sps.max_dec_pic_buffering.max(1) as usize {
            return Err(invalid("HEVC RPS leaves no room for current picture"));
        }
        let context = va
            .context
            .as_ref()
            .ok_or_else(|| invalid("no VA context"))?;
        let target = *context
            .surfaces
            .iter()
            .find(|surface| !self.dpb.holds(**surface))
            .ok_or_else(|| invalid("no free VA surface for the picture"))?;
        let mut buffers = Buffers::new(&va.display, context.context);
        let picture = picture_params(sps, pps, first, &refs, target)?;
        buffers.add(VA_PICTURE_PARAMETER_BUFFER, &picture)?;
        let mut subsets: Vec<u32> = Vec::new();
        for (index, ((unit, _), header)) in slices.iter().zip(&headers).enumerate() {
            let last = index + 1 == slices.len();
            let mut parameters = slice_params(header, &refs, unit, last)?;
            parameters.entry_offset_to_subset_array = u16::try_from(subsets.len())
                .map_err(|_| invalid("HEVC subset array offset overflow"))?;
            subsets.extend_from_slice(&header.entry_point_offsets);
            buffers.add(VA_SLICE_PARAMETER_BUFFER, &parameters)?;
            buffers.add_bytes(VA_SLICE_DATA_BUFFER, unit)?;
        }
        if !subsets.is_empty() {
            buffers.add_elements(VA_SUBSETS_PARAMETER_BUFFER, &subsets)?;
        }
        buffers.submit(target)?;
        drop(buffers);
        self.dpb.insert(target, poc, sps)?;
        Ok(Some(target))
    }
}
