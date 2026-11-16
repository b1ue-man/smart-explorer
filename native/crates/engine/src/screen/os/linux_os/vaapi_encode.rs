//! VA-API H.264 encoder (BK4, B-24): Intel (iHD, i965) and AMD (Mesa)
//! through libva, on the render node of the probe, as the session account.
//! Low-power entry point where the driver offers it with CBR, else the
//! normal one (`vaapi_probe::choose_encode`). Settings as B-21: CBR with a
//! buffer of one frame, no B pictures (P only, one reference), a key frame
//! only on request (the first picture and every request are IDR). Input:
//! the captured BGRA goes into an RGB surface and the VA video processor
//! converts it to NV12 (B-18) where the device has one, else the CPU's
//! I420 goes into the NV12 surface. A DMA-BUF picture from PipeWire (BK8b)
//! enters through `HwH264Session::import_dmabuf` (import, conversion and
//! scaling on the GPU) and is encoded as `HwInput::Prepared`. Config,
//! surfaces, coded buffer, video processor and DMA-BUF import are the
//! shared layer of `vaapi_encode_base.rs` (BK4b: HEVC and AV1 use it too).
use std::io;
use std::sync::Arc;

use super::hw_encode_common::{
    avc_level_ceiling, avc_level_for, HwH264Encoder, HwH264Session, HwInput, RateParams,
};
use super::vaapi_encode_base::{Deliver, EncodeSetup, VaEncodeBase};
use super::vaapi_ffi::{
    VaDisplay, VA_ENC_PACKED_HEADER_TYPE_PICTURE, VA_ENC_PACKED_HEADER_TYPE_SEQUENCE,
    VA_ENC_PICTURE_PARAMETER_BUFFER, VA_ENC_SEQUENCE_PARAMETER_BUFFER,
    VA_ENC_SLICE_PARAMETER_BUFFER,
};
use super::vaapi_h264_headers::{
    picture_params, pps, sequence_params, slice_params, sps, Layout, PictureState, MAX_FRAME_NUM,
};
use super::vaapi_probe::{VaDevice, VaEncodeCaps};
use super::vaapi_vpp::{Buffers, DmaBufFrame};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::EncoderFactory;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::negotiate::{CodecChoice, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::codec_ceiling_bps;

/// Picture order count is 2 × frames since the IDR; an IDR comes before
/// the count could leave `i32`.
pub(super) const MAX_FRAMES_PER_IDR: u32 = 1 << 29;

/// The VA-API encoder of one device.
pub(super) struct VaEncoderFactory {
    pub device: Arc<VaDevice>,
}

impl VaEncoderFactory {
    pub(super) fn name_of(device: &VaDevice) -> String {
        format!("vaapi-{}", device.node_name())
    }

    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        let caps = self
            .device
            .encode
            .iter()
            .find(|caps| Some(caps.profile) == choice.profile)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "VA-API lacks this H.264 profile",
                )
            })?;
        let fits = size.width <= caps.max.width && size.height <= caps.max.height;
        if size != size.even() || !fits || size.width < 16 || size.height < 16 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("VA-API cannot encode {}x{}", size.width, size.height),
            ));
        }
        let api = super::vaapi_ffi::load().map_err(io::Error::other)?;
        let display = VaDisplay::open(api, &self.device.node)?;
        let session = VaH264Session::open(display, caps, self.device.video_proc, size)?;
        let name = self.name();
        let encoder = HwH264Encoder::new(session, name, size, fps, grant);
        // Merge W2: the bit rate up to the pairing's level and the level the
        // session writes (`level_ceiling`), no fixed ceiling.
        Ok(Box::new(
            encoder.with_ceiling(codec_ceiling_bps(choice, size, fps)),
        ))
    }
}

impl EncoderFactory for VaEncoderFactory {
    fn name(&self) -> String {
        Self::name_of(&self.device)
    }

    fn offers(&self) -> Vec<CodecOffer> {
        self.device
            .encode
            .iter()
            .map(|caps| CodecOffer::h264(caps.profile, true).with_max(caps.max))
            .collect()
    }

    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, None)
    }

    fn open_admitted(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, Some(grant))
    }
}

/// One VA-API H.264 encoding session of a fixed size on the shared layer.
pub(super) struct VaH264Session {
    caps: VaEncodeCaps,
    layout: Layout,
    level: u8,
    idr_pic_id: u16,
    frame_num: u32,
    frames_since_idr: u32,
    /// Index (1 or 2) of the reconstructed surface of the last picture.
    last_recon: Option<usize>,
    rate: Option<RateParams>,
    base: VaEncodeBase,
}

impl VaH264Session {
    fn open(
        display: VaDisplay,
        caps: &VaEncodeCaps,
        video_proc: bool,
        size: FrameSize,
    ) -> io::Result<Self> {
        let layout = Layout::new(size);
        let setup = EncodeSetup {
            va_profile: caps.va_profile,
            entrypoint: caps.entrypoint,
            packed_headers: caps.packed_headers,
            size,
            surface: layout.coded(),
            recon: 2,
            video_proc,
        };
        Ok(Self {
            caps: caps.clone(),
            layout,
            level: 0,
            idr_pic_id: 0,
            frame_num: 0,
            frames_since_idr: 0,
            last_recon: None,
            rate: None,
            base: VaEncodeBase::open(display, &setup)?,
        })
    }

    /// Encodes the picture already in the input surface.
    fn encode_surface(
        &mut self,
        idr: bool,
        rate: &RateParams,
        deliver: Deliver<'_>,
    ) -> io::Result<()> {
        let idr = idr || self.last_recon.is_none() || self.frames_since_idr >= MAX_FRAMES_PER_IDR;
        if idr {
            self.idr_pic_id = self.idr_pic_id.wrapping_add(1);
            self.frame_num = 0;
            self.frames_since_idr = 0;
            self.level = avc_level_for(self.layout.size, rate.fps);
        }
        let recon = if self.last_recon == Some(1) && !idr {
            2
        } else {
            1
        };
        let surfaces = &self.base.surfaces;
        let state = PictureState {
            idr,
            frame_num: self.frame_num,
            frames_since_idr: self.frames_since_idr,
            idr_pic_id: self.idr_pic_id,
            recon: surfaces[recon],
            reference: self.last_recon.map(|last| surfaces[last]),
            coded: self.base.coded,
        };
        let mut buffers = Buffers::new(&self.base.display, self.base.context);
        if idr {
            let seq = sequence_params(&self.layout, self.level, rate);
            buffers.add(VA_ENC_SEQUENCE_PARAMETER_BUFFER, &seq)?;
            if self.caps.packed_headers != 0 {
                let profile = self.caps.profile;
                for (kind, nal) in [
                    (
                        VA_ENC_PACKED_HEADER_TYPE_SEQUENCE,
                        sps(&self.layout, profile, self.level),
                    ),
                    (VA_ENC_PACKED_HEADER_TYPE_PICTURE, pps(profile)),
                ] {
                    let bits = (nal.len() * 8) as u32;
                    VaEncodeBase::add_packed(&mut buffers, kind, &nal, bits)?;
                }
            }
        }
        if idr || self.rate != Some(*rate) {
            VaEncodeBase::add_rate(&mut buffers, rate, !idr && self.rate.is_some())?;
        }
        if idr {
            self.base.add_quality(&mut buffers)?;
        }
        let picture = picture_params(&state, self.caps.profile);
        buffers.add(VA_ENC_PICTURE_PARAMETER_BUFFER, &picture)?;
        let slice = slice_params(&state, &self.layout);
        buffers.add(VA_ENC_SLICE_PARAMETER_BUFFER, &slice)?;
        buffers.submit(self.base.input())?;
        drop(buffers);
        self.base.finish(deliver)?;
        self.rate = Some(*rate);
        self.last_recon = Some(recon);
        self.frame_num = (self.frame_num + 1) % MAX_FRAME_NUM;
        self.frames_since_idr += 1;
        Ok(())
    }
}

impl HwH264Session for VaH264Session {
    fn wants_bgra(&self) -> bool {
        self.base.wants_bgra()
    }

    /// MaxBR of the level `encode_surface` writes (`avc_level_for`).
    fn level_ceiling(&self, fps: u32) -> Option<u64> {
        avc_level_ceiling(self.layout.size, fps, self.caps.profile)
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
