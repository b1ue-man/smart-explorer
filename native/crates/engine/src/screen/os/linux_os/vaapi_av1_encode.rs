//! VA-API AV1 encoding session (plan 2026-11-14 „Bildkette“ BK4b, B-24
//! with B-21 as BK4's H.264): Main 8-bit 4:2:0 where the driver reports
//! `VAProfileAV1Profile0` with `VAEntrypointEncSlice`/`EncSliceLP` and
//! CBR; a key frame first and on request, inter frames with the previous
//! frame as the only reference. Input, video processor, DMA-BUF import
//! (BK8b) and coded read-back are the shared layer of
//! `vaapi_encode_base.rs`, as for H.264 and HEVC. The sequence and frame
//! header OBUs go in as packed headers where the driver takes them
//! (`VA_ENC_PACKED_HEADER_SEQUENCE`, `…_PICTURE`); the output becomes one
//! temporal unit in the low-overhead format with a temporal delimiter first
//! and the sequence header on every key frame (`ordered_temporal_unit`),
//! which is what `wire_binary.rs`, `hevc_av1_bits.rs` and the viewers'
//! decoders take.
use std::io;

use super::hw_encode_common::{HwH264Session, HwInput, RateParams};
use super::vaapi_av1_bits::ordered_temporal_unit;
use super::vaapi_av1_headers::{
    frame_header, picture_params, sequence_header, sequence_params, tile_group, Av1Layout,
    Av1Picture, Av1Tools,
};
use super::vaapi_encode::MAX_FRAMES_PER_IDR;
use super::vaapi_encode_base::{Deliver, EncodeSetup, VaEncodeBase};
use super::vaapi_ffi::{
    VaDisplay, VA_ENC_PACKED_HEADER_PICTURE, VA_ENC_PACKED_HEADER_SEQUENCE,
    VA_ENC_PACKED_HEADER_TYPE_PICTURE, VA_ENC_PACKED_HEADER_TYPE_SEQUENCE,
    VA_ENC_PICTURE_PARAMETER_BUFFER, VA_ENC_SEQUENCE_PARAMETER_BUFFER,
    VA_ENC_SLICE_PARAMETER_BUFFER,
};
use super::vaapi_probe::VaVideoEncodeCaps;
use super::vaapi_vpp::{Buffers, DmaBufFrame};
use crate::screen::core::frame_size_caps::av1_level_for;
use crate::screen::core::pixel_convert::FrameSize;

/// The packed headers this session writes where the driver takes them.
pub(super) const AV1_PACKED: u32 = VA_ENC_PACKED_HEADER_SEQUENCE | VA_ENC_PACKED_HEADER_PICTURE;

/// `seq_level_idx` for `size` at `fps` (Annex A.3), 31 ("maximum
/// parameters") when no level carries it.
pub(super) fn av1_level_idx(size: FrameSize, fps: u32) -> u8 {
    av1_level_for(size, fps.max(1)).map_or(31, |level| level.seq_level_idx)
}

/// One VA-API AV1 encoding session of a fixed size.
pub(super) struct VaAv1Session {
    layout: Av1Layout,
    tools: Av1Tools,
    packed: u32,
    level: u8,
    /// The sequence header OBU of the running key frame.
    sequence: Vec<u8>,
    frames_since_key: u32,
    last_recon: Option<usize>,
    rate: Option<RateParams>,
    base: VaEncodeBase,
}

impl VaAv1Session {
    pub(super) fn open(
        display: VaDisplay,
        caps: &VaVideoEncodeCaps,
        video_proc: bool,
        size: FrameSize,
        quality: Option<u8>,
    ) -> io::Result<Self> {
        let tools = Av1Tools::new(caps.features, caps.blocks, caps.ext2, caps.prediction);
        let layout = Av1Layout::new(size, &tools);
        let packed = if caps.packed & AV1_PACKED == AV1_PACKED {
            AV1_PACKED
        } else {
            0
        };
        let setup = EncodeSetup {
            va_profile: caps.va_profile,
            entrypoint: caps.entrypoint,
            packed_headers: packed,
            size,
            surface: layout.surface(),
            recon: 2,
            video_proc,
            quality,
        };
        Ok(Self {
            layout,
            tools,
            packed,
            level: 0,
            sequence: Vec::new(),
            frames_since_key: 0,
            last_recon: None,
            rate: None,
            base: VaEncodeBase::open(display, &setup)?,
        })
    }

    fn encode_surface(
        &mut self,
        key: bool,
        rate: &RateParams,
        deliver: Deliver<'_>,
    ) -> io::Result<()> {
        let key = key || self.last_recon.is_none() || self.frames_since_key >= MAX_FRAMES_PER_IDR;
        if key {
            self.frames_since_key = 0;
            self.level = av1_level_idx(self.layout.size, rate.fps);
            self.sequence = sequence_header(&self.layout, &self.tools, self.level);
        }
        let recon = if self.last_recon == Some(1) && !key {
            2
        } else {
            1
        };
        let surfaces = &self.base.surfaces;
        let picture = Av1Picture {
            key,
            order_hint: (self.frames_since_key % 256) as u8,
            recon: surfaces[recon],
            reference: self.last_recon.map(|last| surfaces[last]),
            coded: self.base.coded,
        };
        let (layout, tools) = (&self.layout, &self.tools);
        let header = frame_header(layout, tools, &picture);
        let mut buffers = Buffers::new(&self.base.display, self.base.context);
        if key {
            let seq = sequence_params(tools, self.level, rate);
            buffers.add(VA_ENC_SEQUENCE_PARAMETER_BUFFER, &seq)?;
        }
        if key || self.rate != Some(*rate) {
            self.base
                .add_rate(&mut buffers, rate, !key && self.rate.is_some())?;
        }
        if key {
            self.base.add_quality(&mut buffers)?;
        }
        let mut params = picture_params(layout, tools, &picture, &header);
        if self.packed == 0 {
            params.bit_offset_qindex = 0;
            params.bit_offset_loopfilter_params = 0;
            params.bit_offset_cdef_params = 0;
            params.size_in_bits_cdef_params = 0;
            params.byte_offset_frame_hdr_obu_size = 0;
            params.size_in_bits_frame_hdr_obu = 0;
        }
        buffers.add(VA_ENC_PICTURE_PARAMETER_BUFFER, &params)?;
        if key && self.packed & VA_ENC_PACKED_HEADER_SEQUENCE != 0 {
            let bits = (self.sequence.len() * 8) as u32;
            let kind = VA_ENC_PACKED_HEADER_TYPE_SEQUENCE;
            VaEncodeBase::add_packed(&mut buffers, kind, &self.sequence, bits)?;
        }
        if self.packed & VA_ENC_PACKED_HEADER_PICTURE != 0 {
            let bits = (header.bytes.len() * 8) as u32;
            let kind = VA_ENC_PACKED_HEADER_TYPE_PICTURE;
            VaEncodeBase::add_packed(&mut buffers, kind, &header.bytes, bits)?;
        }
        buffers.add(VA_ENC_SLICE_PARAMETER_BUFFER, &tile_group(layout))?;
        buffers.submit(self.base.input())?;
        drop(buffers);
        let sequence = &self.sequence;
        self.base.finish(&mut |segments| {
            let unit = ordered_temporal_unit(&segments.concat(), key, sequence)?;
            deliver(&[&unit])
        })?;
        self.rate = Some(*rate);
        self.last_recon = Some(recon);
        self.frames_since_key += 1;
        Ok(())
    }
}

impl HwH264Session for VaAv1Session {
    fn wants_bgra(&self) -> bool {
        self.base.wants_bgra()
    }

    /// MaxBitrate (Main tier) of the level `encode_surface` writes
    /// (`av1_level_idx`; none for level 31, "maximum parameters"; merge W2).
    fn level_ceiling(&self, fps: u32) -> Option<u64> {
        av1_level_for(self.layout.size, fps.max(1))
            .map(|level| level.main_kbps.saturating_mul(1000))
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
        key: bool,
        rate: &RateParams,
        deliver: Deliver<'_>,
    ) -> io::Result<()> {
        self.base.upload(input)?;
        self.encode_surface(key, rate, deliver)
    }
}
