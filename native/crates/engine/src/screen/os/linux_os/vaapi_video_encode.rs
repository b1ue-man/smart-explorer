//! The VA-API HEVC and AV1 encoder factories (plan 2026-11-14 „Bildkette“
//! BK4b, B-24, B-30): one per render node and codec, named
//! `vaapi-hevc-renderD128`/`vaapi-av1-renderD128` so the host process's
//! `refused` list keeps a codec that failed apart from the others (as
//! BK6's `nvenc-hevc`/`nvenc-av1`). Each offers 4:2:0 Main (`profile`
//! absent on the wire, B-32) up to the driver's largest picture; opening
//! checks the size, opens the render node as the account of the process
//! and wraps the session in BK4's `HwH264Encoder` with `for_codec`, which
//! admits every output copy, marks key frames, caches the parameter sets
//! and checks them against the frame contract.
use std::io;
use std::sync::Arc;

use super::hw_encode_common::{HwH264Encoder, HwH264Session};
use super::vaapi_av1_encode::VaAv1Session;
use super::vaapi_ffi::VaDisplay;
use super::vaapi_hevc_encode::VaHevcSession;
use super::vaapi_probe::{VaDevice, VaVideoEncodeCaps};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::EncoderFactory;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::codec_ceiling_bps;

/// Smallest side the VA HEVC and AV1 encoders are opened with (one CTB or
/// superblock of 64; assumption A-VA-VIDEO-MIN, as A-BK6-NV-MIN).
pub(super) const MIN_SIDE: u32 = 64;

/// The VA-API encoder of one codec on one device.
pub(super) struct VaVideoEncoderFactory {
    pub device: Arc<VaDevice>,
    pub caps: VaVideoEncodeCaps,
}

impl VaVideoEncoderFactory {
    pub(super) fn name_of(device: &VaDevice, codec: CodecKind) -> String {
        format!("vaapi-{}-{}", codec.key(), device.node_name())
    }

    /// Whether `size` can be opened: even, within the driver's limit and
    /// at least `MIN_SIDE` on each side.
    pub(super) fn fits(&self, size: FrameSize) -> bool {
        let max = self.caps.max;
        size == size.even()
            && size.width <= max.width
            && size.height <= max.height
            && size.width >= MIN_SIDE
            && size.height >= MIN_SIDE
    }

    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        let codec = self.caps.codec;
        if choice.codec != codec || choice.profile.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "VA-API offers 4:2:0 Main of HEVC and AV1 only",
            ));
        }
        if !self.fits(size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("VA-API cannot encode {}x{}", size.width, size.height),
            ));
        }
        let api = super::vaapi_ffi::load().map_err(io::Error::other)?;
        let display = VaDisplay::open(api, &self.device.node)?;
        let video_proc = self.device.video_proc;
        // P9: the constant quality of sharp mode for an admitted session.
        let quality = grant
            .as_ref()
            .and_then(|_| crate::screen::core::rate_control::model::session_quality());
        match codec {
            CodecKind::Hevc => {
                let session = VaHevcSession::open(display, &self.caps, video_proc, size, quality)?;
                Ok(self.wrap(session, (choice, size, fps), grant))
            }
            CodecKind::Av1 => {
                let session = VaAv1Session::open(display, &self.caps, video_proc, size, quality)?;
                Ok(self.wrap(session, (choice, size, fps), grant))
            }
            CodecKind::H264 | CodecKind::Tiles => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "VA-API video factory carries HEVC and AV1 only",
            )),
        }
    }

    /// The encoder around `session`; its bit rate up to the pairing's level
    /// and the level the session writes (merge W2, no fixed ceiling).
    fn wrap<S: HwH264Session + 'static>(
        &self,
        session: S,
        (choice, size, fps): (&CodecChoice, FrameSize, u32),
        grant: Option<CaptureGrant>,
    ) -> Box<dyn ScreenEncoder> {
        let encoder = HwH264Encoder::new(session, self.name(), size, fps, grant);
        let ceiling = codec_ceiling_bps(choice, size, fps);
        Box::new(encoder.for_codec(self.caps.codec).with_ceiling(ceiling))
    }
}

impl EncoderFactory for VaVideoEncoderFactory {
    fn name(&self) -> String {
        Self::name_of(&self.device, self.caps.codec)
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::video(self.caps.codec, false, true).with_max(self.caps.max)]
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
