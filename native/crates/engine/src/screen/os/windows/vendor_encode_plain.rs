//! The vendor encoders without admission (plan 2026-11-17 „Windows-Bild“
//! P3 (b)): the run-time check and `bench --local` open NVENC and AMF on a
//! device of their own (`d3d_input::own_for_vendor`) and hand BGRA
//! pictures, copied up on the CPU (`GpuInput::upload_bgra`); the access
//! unit is collected in plain bytes, with the same rules as the admitted
//! path (`append_unit`, `finish`). Split from `vendor_encode.rs`.
use std::io;

use super::{append_unit, VendorEncoder, VendorPicture, VendorSession};
use crate::screen::core::capture_image::AdmittedBytes;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::frame_size_caps::check_video_dimensions;
use crate::screen::core::host_backend::EncodedFrame;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::wire::invalid;
use crate::screen::core::wire_binary::MAX_RECORD_BYTES;
use crate::screen::os::mf_encoder::d3d_input::GpuInput;

/// Where `append_unit` writes: the admitted bytes of a session, or the
/// plain bytes of a picture without admission (P3).
pub(in crate::screen::os) trait UnitSink {
    fn written(&self) -> usize;
    fn put(&mut self, bytes: &[u8]) -> io::Result<()>;
}

impl UnitSink for AdmittedBytes {
    fn written(&self) -> usize {
        self.len()
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes)
    }
}

impl UnitSink for Vec<u8> {
    fn written(&self) -> usize {
        self.len()
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

impl<S: VendorSession> VendorEncoder<S> {
    /// P3: a session without admission on a device of its own (`gpu`),
    /// for the run-time check and `bench --local`; pictures go through
    /// `encode`.
    pub(crate) fn plain(
        session: S,
        name: &str,
        codec: CodecKind,
        size: FrameSize,
        fps: u32,
        gpu: GpuInput,
    ) -> Self {
        Self {
            session,
            name: name.into(),
            codec,
            size,
            grant: None,
            gpu,
            plain_fps: fps.max(1),
            frames: 0,
            rate: None,
            ceiling_bps: u64::from(u32::MAX),
            refresh: false,
            // The run-time check and `bench --local` measure the CBR path.
            quality: None,
            parameter_sets: None,
        }
    }

    /// One picture without admission (`ScreenEncoder::encode`).
    pub(super) fn encode_plain(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        if self.grant.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "vendor encoder requires encode_captured",
            ));
        }
        let texture = self.gpu.upload_bgra(&frame.view())?;
        let rate = self.rate(target_bps, self.plain_fps);
        let idr = keyframe || self.frames == 0;
        let picture = VendorPicture {
            texture: &texture,
            frame_id: self.frames.saturating_add(1),
            idr,
        };
        let (codec, kept) = (self.codec, &mut self.parameter_sets);
        let mut bytes: Vec<u8> = Vec::new();
        let mut marked = false;
        let coded = self.session.encode(picture, &rate, &mut |unit| {
            marked |= append_unit(&mut bytes, MAX_RECORD_BYTES, codec, kept, unit)?;
            Ok(())
        });
        self.gpu.finish();
        let keyframe = coded? || marked;
        if bytes.is_empty() {
            return Err(invalid("vendor encoder gave no picture for the frame"));
        }
        if idr && !keyframe {
            return Err(invalid("vendor encoder gave no key frame on request"));
        }
        check_video_dimensions(self.codec, &bytes, self.size, FrameLimits::RESOURCE)?;
        self.frames = self.frames.saturating_add(1);
        Ok(EncodedFrame {
            records: vec![bytes],
            keyframe,
        })
    }
}

/// Codex round 6, finding 3: a session whose late picture did not finish
/// keeps the ring slot of its input (`GpuInput::abandon`); the capture then
/// never writes into a texture the driver may still read.
impl<S: VendorSession> Drop for VendorEncoder<S> {
    fn drop(&mut self) {
        if !self.session.end_late() {
            self.gpu.abandon();
        }
    }
}
