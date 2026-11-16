//! NVENC HEVC and AV1 for Linux hosts with an NVIDIA GPU (plan 2026-11-14
//! „Bildkette“ BK6, B-30, on the session of BK4's `nvenc_encode.rs`): the
//! factory. GUIDs, the views on the codec union and their settings are
//! shared with Windows since merge W2 (`native_ffi/nvenc_codecs.rs`,
//! re-exported here): HEVC Annex B (`IDR_W_RADL` on request), AV1 OBUs in
//! the low-overhead format (`outputAnnexBFormat` 0), 8-bit 4:2:0, BT.709
//! limited range, the parameter sets at every key frame.
use std::io;

use super::hw_encode_common::HwH264Encoder;
use super::nvenc_encode::NvencSession;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::EncoderFactory;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::codec_ceiling_bps;

pub(super) use crate::screen::os::nvenc_shared::codecs::{
    encode_guid, factory_name, NV_ENC_CODEC_AV1_GUID, NV_ENC_CODEC_HEVC_GUID,
};

/// The NVENC encoder of HEVC or AV1 (4:2:0, Main) of this system.
pub(super) struct NvencVideoFactory {
    pub codec: CodecKind,
    pub max: FrameSize,
}

impl NvencVideoFactory {
    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        if choice.codec != self.codec || choice.profile.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "NVENC offers 4:2:0 Main of HEVC and AV1 only",
            ));
        }
        let fits = size.width <= self.max.width && size.height <= self.max.height;
        if size != size.even() || !fits || size.width < 64 || size.height < 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("NVENC cannot encode {}x{}", size.width, size.height),
            ));
        }
        let session = NvencSession::open_codec(size, fps, self.codec)?;
        let encoder = HwH264Encoder::new(session, factory_name(self.codec), size, fps, grant);
        // Merge W2: the bit rate up to the pairing's level, no fixed ceiling.
        let ceiling = codec_ceiling_bps(choice, size, fps);
        Ok(Box::new(
            encoder.for_codec(self.codec).with_ceiling(ceiling),
        ))
    }
}

impl EncoderFactory for NvencVideoFactory {
    fn name(&self) -> String {
        factory_name(self.codec)
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::video(self.codec, false, true).with_max(self.max)]
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
