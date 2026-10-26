//! The H.264 decoder of Media Foundation (a synchronous MFT; SCR-M3,
//! decision C2): Annex B access units in, NV12 out (the first NV12 type the
//! decoder offers), converted to BGRA with `nv12_to_bgra` (BT.709) or, after
//! `prefer_yuv(true)` (SCR-M7, the viewer's GPU path), kept as planes without
//! the conversion. The decoder's buffer may be taller than the picture (rows padded to 16); its
//! row pitch follows from the buffer length, and only the picture's area is
//! converted. A changed stream (`MF_E_TRANSFORM_STREAM_CHANGE`) selects NV12
//! again. Low latency through `MF_LOW_LATENCY`.
use std::io;

use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFTransform, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MF_E_NOTACCEPTING,
    MF_LOW_LATENCY, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE,
};

use super::mf_common::ensure_com;
use super::mf_encoder_sync::{stop, UNITS_PER_SECOND};
use super::mf_media::{
    input_sample, process_output_bounded, sample_bytes_bounded, video_type, Output,
};
use crate::screen::core::decoded_frame::{DecodedFrame, DecoderFallback, FrameIdentities};
use crate::screen::core::host_backend::{Chroma, ScreenDecoder, YuvPicture};
use crate::screen::core::pixel_convert::{nv12_to_bgra, BgraImage, FrameSize, Nv12View, YuvMatrix};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

/// Time stamps of the input samples: 30 frames per second.
const FRAME_UNITS: i64 = UNITS_PER_SECOND / 30;
/// Output types the decoder may list before NV12.
const MAX_TYPES: u32 = 64;

pub(super) struct MfDecoder {
    transform: IMFTransform,
    name: String,
    /// The picture of the stream head.
    size: FrameSize,
    /// The decoder's picture with its padding.
    allocated: FrameSize,
    limits: crate::screen::core::frame_budget::FrameLimits,
    picture: BgraImage,
    frames: i64,
    identities: FrameIdentities,
    decoded: Option<DecodedFrame>,
    /// `prefer_yuv(true)`: keep the planes in `planes`, do not convert.
    yuv: bool,
    planes: Option<YuvPicture>,
}

// SAFETY: one thread at a time (the decoder is borrowed mutably); the
// transform is a free-threaded object of the multithreaded apartment.
unsafe impl Send for MfDecoder {}

impl MfDecoder {
    pub(super) fn new(
        transform: IMFTransform,
        name: String,
        header: &GopHeader,
    ) -> io::Result<Self> {
        let limits = header.frame_contract.map_or(
            crate::screen::core::frame_budget::FrameLimits::LEGACY,
            |c| c.limits,
        );
        limits.check(header.size)?;
        let input = video_type(&MFVideoFormat_H264, header.size, 30)?;
        // SAFETY: attribute write and type selection on the activated
        // transform; low latency is optional.
        unsafe {
            if let Ok(attributes) = transform.GetAttributes() {
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            transform.SetInputType(0, &input, 0)?;
        }
        let mut decoder = Self {
            transform,
            name,
            size: header.size,
            allocated: header.size,
            picture: BgraImage::new_with_limits(header.size, limits)?,
            limits,
            frames: 0,
            identities: FrameIdentities::default(),
            decoded: None,
            yuv: false,
            planes: None,
        };
        decoder.select_nv12()?;
        // SAFETY: notifications on the configured transform.
        unsafe {
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        Ok(decoder)
    }

    /// Selects the first NV12 output type and notes its padded size.
    fn select_nv12(&mut self) -> io::Result<()> {
        for index in 0..MAX_TYPES {
            // SAFETY: queries and selection of the transform's output type.
            let Ok(media) = (unsafe { self.transform.GetOutputAvailableType(0, index) }) else {
                break;
            };
            let subtype = unsafe { media.GetGUID(&MF_MT_SUBTYPE) };
            if !subtype.is_ok_and(|subtype| subtype == MFVideoFormat_NV12) {
                continue;
            }
            let packed = unsafe { media.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap_or(0);
            let padded = FrameSize::new((packed >> 32) as u32, packed as u32);
            let allocated = if padded.width == 0 && padded.height == 0 {
                self.size
            } else {
                padded
            };
            if allocated.width < self.size.width
                || allocated.height < self.size.height
                || allocated.width > self.size.width.saturating_add(15)
                || allocated.height > self.size.height.saturating_add(15)
            {
                return Err(io::Error::other(
                    "MF decoder geometry contradicts the stream",
                ));
            }
            self.limits.check(allocated)?;
            unsafe {
                self.transform.SetOutputType(0, &media, 0)?;
            }
            self.allocated = allocated;
            return Ok(());
        }
        Err(io::Error::other(
            "the Media Foundation decoder offers no NV12 output",
        ))
    }

    /// Converts one NV12 output sample into the picture.
    fn convert(&mut self, sample: &IMFSample) -> io::Result<()> {
        let max = self.limits.check(self.allocated)?;
        let bytes = sample_bytes_bounded(sample, max)?;
        let rows = self.allocated.height as usize;
        let pitch = bytes
            .len()
            .checked_mul(2)
            .ok_or_else(|| io::Error::other("NV12 size overflow"))?
            / (rows * 3).max(1);
        let luma = pitch * rows;
        if pitch < self.size.width as usize || bytes.len() < luma {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decoded NV12 picture is smaller than its type says",
            ));
        }
        let view = Nv12View::new_with_limits(
            self.size,
            &bytes[..luma],
            pitch,
            &bytes[luma..],
            pitch,
            self.limits,
        )?;
        if self.yuv {
            let planes = self.planes.get_or_insert_with(|| YuvPicture {
                y: Vec::new(),
                y_stride: pitch,
                uv: Vec::new(),
                uv_stride: pitch,
                chroma: Chroma::Nv12,
                matrix: YuvMatrix::Bt709,
            });
            planes.y.clear();
            planes.y.extend_from_slice(&bytes[..luma]);
            planes.uv.clear();
            planes.uv.extend_from_slice(&bytes[luma..]);
            planes.y_stride = pitch;
            planes.uv_stride = pitch;
            return Ok(());
        }
        nv12_to_bgra(&view, YuvMatrix::Bt709, &mut self.picture)
    }

    /// Takes every output ready now; whether a picture came.
    fn drain(&mut self) -> io::Result<bool> {
        let mut shown = false;
        let fallback = self.allocated.pixels() as usize * 3 / 2;
        loop {
            match process_output_bounded(
                &self.transform,
                fallback,
                self.limits.check(self.allocated)?,
            )? {
                Output::Sample(out) => {
                    // SAFETY: query on the output sample owned by this call. A drain
                    // before ProcessInput accepts the next AU still names its old AU.
                    let time = unsafe { out.GetSampleTime() }
                        .map_err(|_| DecoderFallback::OutputIdentityUnavailable.error())?;
                    let token = u64::try_from(time)
                        .map_err(|_| DecoderFallback::OutputIdentityUnavailable.error())?;
                    let identity = self.identities.take(token)?;
                    self.convert(&out)?;
                    self.decoded = Some(identity);
                    shown = true;
                }
                Output::NeedInput => return Ok(shown),
                Output::StreamChange => self.select_nv12()?,
            }
        }
    }
}

impl ScreenDecoder for MfDecoder {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        ensure_com();
        self.decoded = None;
        crate::screen::core::frame_size_caps::check_avc_dimensions(
            &record.payload,
            self.size,
            self.limits,
        )?;
        let time = self
            .frames
            .checked_mul(FRAME_UNITS)
            .ok_or_else(|| io::Error::other("MF decoder timestamp overflow"))?;
        let next = self
            .frames
            .checked_add(1)
            .ok_or_else(|| io::Error::other("MF decoder frame counter overflow"))?;
        let sample = input_sample(&record.payload, time, FRAME_UNITS)?;
        let mut shown = false;
        // SAFETY: one owned input sample for stream 0.
        if let Err(error) = unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            if error.code() != MF_E_NOTACCEPTING {
                return Err(io::Error::other(error));
            }
            shown = self.drain()?;
            // SAFETY: as above, after the pending output was taken.
            unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        }
        // Register only after acceptance: earlier drained output cannot claim this AU.
        self.identities
            .insert(time as u64, DecodedFrame::from_record(record))?;
        self.frames = next;
        Ok(self.drain()? || shown)
    }

    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }

    fn picture(&self) -> &BgraImage {
        &self.picture
    }

    fn prefer_yuv(&mut self, on: bool) {
        self.yuv = on;
        if !on {
            self.planes = None;
        }
    }

    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.planes.clone().filter(|_| self.yuv)
    }
}

impl Drop for MfDecoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}

#[cfg(test)]
mod tests {
    use crate::screen::core::host_backend::{ScreenDecoder, ScreenEncoder};
    use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
    use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
    use crate::screen::core::synthetic::{read_barcode, render, BarcodeRead};
    use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};

    /// The software and every hardware encoder of Media Foundation against
    /// its decoder, both profiles: 30 frames of the test picture must come
    /// back with their barcodes. Only where Media Foundation runs; the
    /// GitHub runners have none (K6), the suite reports it as not run there.
    #[test]
    #[ignore = "needs Media Foundation; the suite runs it where `se screen codec` lists an MFT"]
    fn screen_task_mf_encode_decode() {
        assert!(super::super::mf_common::available(), "no Media Foundation");
        let (encoders, decoders) = super::super::mf_common::codecs();
        let size = FrameSize::new(640, 360);
        let decoder_factory = decoders.first().expect("an MF decoder");
        for factory in &encoders {
            for profile in [H264Profile::ConstrainedBaseline, H264Profile::High] {
                let hardware = factory.offers().iter().any(|offer| offer.hardware);
                let choice = CodecChoice {
                    codec: CodecKind::H264,
                    profile: Some(profile),
                    encoder_hardware: hardware,
                    decoder_hardware: false,
                    max_size: FrameSize::MAX,
                    max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
                    max_macroblocks_per_second: None,
                };
                let mut encoder = factory.open(&choice, size, 30).unwrap();
                let header = GopHeader {
                    family_layout_revision: None,
                    frame_contract: None,
                    gop_id: 1,
                    kind: StreamKind::Video,
                    codec: CodecKind::H264,
                    profile: Some(profile),
                    size,
                };
                let mut decoder = decoder_factory.open(&header).unwrap();
                let mut image = BgraImage::new(size).unwrap();
                let mut good = 0;
                for frame_id in 1..=30u64 {
                    render(frame_id, &mut image).unwrap();
                    let encoded = encoder.encode(&image, frame_id == 1, 2_000_000).unwrap();
                    if frame_id == 1 {
                        assert!(encoded.keyframe, "the first frame is a key frame");
                    }
                    for payload in encoded.records {
                        let flags = if encoded.keyframe { FLAG_KEYFRAME } else { 0 };
                        let record = FrameRecord {
                            frame_id,
                            capture_ts_us: 0,
                            flags,
                            payload,
                        };
                        if decoder.decode(&record).unwrap() {
                            let output = decoder
                                .decoded_frame()
                                .expect("decoded MF picture identity");
                            let read = read_barcode(&decoder.picture().view(), size).unwrap();
                            if matches!(read, BarcodeRead::Valid(id) if id == output.frame_id) {
                                good += 1;
                            }
                        }
                    }
                }
                assert!(good >= 25, "{}: {good} of 30 frames back", encoder.name());
            }
        }
    }
}
