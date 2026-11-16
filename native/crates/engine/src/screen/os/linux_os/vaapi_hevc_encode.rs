//! VA-API HEVC encoding session (plan 2026-11-14 „Bildkette“ BK4b, B-24
//! with B-21 as BK4's H.264): Main 8-bit 4:2:0 on the entry point the probe
//! chose (low power where it offers CBR), CBR with a buffer of one frame,
//! P pictures with one reference (low-delay B where the driver wants both
//! lists filled), IDR only first and on request. Input, video processor,
//! DMA-BUF import (BK8b) and coded read-back are the shared layer of
//! `vaapi_encode_base.rs`, exactly as for H.264. Parameter sets go into one
//! packed sequence header and the slice header into a packed slice header
//! wherever the driver takes them (`VA_ENC_PACKED_HEADER_SEQUENCE`,
//! `…_SLICE`); a driver without them writes its own, and an IDR without
//! VPS/SPS/PPS fails in `HwH264Encoder` so the next encoder takes over.
use std::io;

use super::hw_encode_common::{HwH264Session, HwInput, RateParams};
use super::vaapi_encode::MAX_FRAMES_PER_IDR;
use super::vaapi_encode_base::{Deliver, EncodeSetup, VaEncodeBase};
use super::vaapi_ffi::{
    VaDisplay, VA_ENC_PACKED_HEADER_PICTURE, VA_ENC_PACKED_HEADER_SEQUENCE,
    VA_ENC_PACKED_HEADER_SLICE, VA_ENC_PACKED_HEADER_TYPE_PICTURE,
    VA_ENC_PACKED_HEADER_TYPE_SEQUENCE, VA_ENC_PACKED_HEADER_TYPE_SLICE,
    VA_ENC_PICTURE_PARAMETER_BUFFER, VA_ENC_SEQUENCE_PARAMETER_BUFFER,
    VA_ENC_SLICE_PARAMETER_BUFFER,
};
use super::vaapi_hevc_headers::{pps, slice_header, sps, vps, HevcLayout, HevcTools};
use super::vaapi_hevc_params::{picture_params, sequence_params, slice_params, HevcPicture};
use super::vaapi_probe::VaVideoEncodeCaps;
use super::vaapi_vpp::{Buffers, DmaBufFrame};
use crate::screen::core::frame_size_caps::{hevc_level_for, HEVC_LEVELS};
use crate::screen::core::pixel_convert::FrameSize;

/// The packed headers this session writes where the driver takes them.
const PARAM_PACKED: u32 = VA_ENC_PACKED_HEADER_SEQUENCE | VA_ENC_PACKED_HEADER_PICTURE;
pub(super) const HEVC_PACKED: u32 = PARAM_PACKED | VA_ENC_PACKED_HEADER_SLICE;

/// `general_level_idc` for `size` at `fps`: the smallest level of H.265
/// Table A.8/A.9 that carries it, else the highest.
pub(super) fn hevc_level_idc(size: FrameSize, fps: u32) -> u8 {
    hevc_level_for(size, fps.max(1))
        .or_else(|| HEVC_LEVELS.last().copied())
        .map_or(186, |level| level.level_idc)
}

/// One VA-API HEVC encoding session of a fixed size.
pub(super) struct VaHevcSession {
    layout: HevcLayout,
    tools: HevcTools,
    packed: u32,
    level_idc: u8,
    poc: u32,
    /// Index (1 or 2) of the reconstructed surface of the last picture.
    last_recon: Option<usize>,
    rate: Option<RateParams>,
    base: VaEncodeBase,
}

impl VaHevcSession {
    pub(super) fn open(
        display: VaDisplay,
        caps: &VaVideoEncodeCaps,
        video_proc: bool,
        size: FrameSize,
    ) -> io::Result<Self> {
        let layout = HevcLayout::new(size, caps.blocks);
        let packed = (if caps.packed & PARAM_PACKED == PARAM_PACKED {
            PARAM_PACKED
        } else {
            0
        }) | (caps.packed & VA_ENC_PACKED_HEADER_SLICE);
        let setup = EncodeSetup {
            va_profile: caps.va_profile,
            entrypoint: caps.entrypoint,
            packed_headers: packed,
            size,
            surface: layout.surface(),
            recon: 2,
            video_proc,
        };
        Ok(Self {
            layout,
            tools: HevcTools::new(caps.features, caps.prediction),
            packed,
            level_idc: 0,
            poc: 0,
            last_recon: None,
            rate: None,
            base: VaEncodeBase::open(display, &setup)?,
        })
    }

    fn encode_surface(
        &mut self,
        idr: bool,
        rate: &RateParams,
        deliver: Deliver<'_>,
    ) -> io::Result<()> {
        let idr = idr || self.last_recon.is_none() || self.poc >= MAX_FRAMES_PER_IDR;
        if idr {
            self.poc = 0;
            // The level covers the coded size the SPS names (the
            // viewer's `check_video_dimensions` tests that one).
            self.level_idc = hevc_level_idc(self.layout.coded(), rate.fps);
        }
        let recon = if self.last_recon == Some(1) && !idr {
            2
        } else {
            1
        };
        let surfaces = &self.base.surfaces;
        let picture = HevcPicture {
            idr,
            poc: self.poc,
            recon: surfaces[recon],
            reference: self.last_recon.map(|last| surfaces[last]),
            coded: self.base.coded,
        };
        let (layout, tools) = (&self.layout, &self.tools);
        let mut buffers = Buffers::new(&self.base.display, self.base.context);
        if idr {
            let seq = sequence_params(layout, tools, self.level_idc, rate);
            buffers.add(VA_ENC_SEQUENCE_PARAMETER_BUFFER, &seq)?;
            if self.packed & PARAM_PACKED == PARAM_PACKED {
                for nal in [vps(self.level_idc), sps(layout, tools, self.level_idc)] {
                    let bits = (nal.len() * 8) as u32;
                    VaEncodeBase::add_packed(
                        &mut buffers,
                        VA_ENC_PACKED_HEADER_TYPE_SEQUENCE,
                        &nal,
                        bits,
                    )?;
                }
            }
        }
        if idr || self.rate != Some(*rate) {
            VaEncodeBase::add_rate(&mut buffers, rate, !idr && self.rate.is_some())?;
        }
        if idr {
            self.base.add_quality(&mut buffers)?;
        }
        buffers.add(
            VA_ENC_PICTURE_PARAMETER_BUFFER,
            &picture_params(tools, &picture),
        )?;
        if idr && self.packed & PARAM_PACKED == PARAM_PACKED {
            let nal = pps(tools);
            let bits = (nal.len() * 8) as u32;
            VaEncodeBase::add_packed(&mut buffers, VA_ENC_PACKED_HEADER_TYPE_PICTURE, &nal, bits)?;
        }
        if self.packed & VA_ENC_PACKED_HEADER_SLICE != 0 {
            let header = slice_header(tools, &picture);
            let bits = (header.len() * 8) as u32;
            VaEncodeBase::add_packed(&mut buffers, VA_ENC_PACKED_HEADER_TYPE_SLICE, &header, bits)?;
        }
        buffers.add(
            VA_ENC_SLICE_PARAMETER_BUFFER,
            &slice_params(layout, tools, &picture),
        )?;
        buffers.submit(self.base.input())?;
        drop(buffers);
        self.base.finish(deliver)?;
        self.rate = Some(*rate);
        self.last_recon = Some(recon);
        self.poc += 1;
        Ok(())
    }
}

impl HwH264Session for VaHevcSession {
    fn wants_bgra(&self) -> bool {
        self.base.wants_bgra()
    }

    /// MaxBR (Main tier) of the level `encode_surface` writes
    /// (`hevc_level_idc` of the coded size; merge W2).
    fn level_ceiling(&self, fps: u32) -> Option<u64> {
        let idc = hevc_level_idc(self.layout.coded(), fps);
        HEVC_LEVELS
            .iter()
            .find(|level| level.level_idc == idc)
            .map(|level| level.max_br_main.saturating_mul(1000))
    }

    fn imports_dmabuf(&self) -> bool {
        self.base.wants_bgra()
    }

    /// BK8b: the shared layer imports, converts and scales the DMA-BUF.
    fn import_dmabuf(&mut self, frame: &DmaBufFrame) -> io::Result<()> {
        self.base.import_dmabuf(frame)
    }

    fn encode(
        &mut self,
        input: HwInput<'_>,
        idr: bool,
        rate: &RateParams,
        deliver: Deliver<'_>,
    ) -> io::Result<()> {
        self.base.upload(input)?;
        self.encode_surface(idr, rate, deliver)
    }
}
