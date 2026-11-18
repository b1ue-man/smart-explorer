//! The H.264, HEVC and AV1 decoders of Media Foundation (synchronous MFTs;
//! SCR-M3, decision C2; HEVC and AV1 since plan 2026-11-14 „Bildkette“ BK6,
//! where the system has their MFT: an access unit or temporal unit in,
//! `MFVideoFormat_HEVC`/`MFVideoFormat_AV1` as input subtype, the DXVA
//! profiles HEVC Main and AV1 profile 0 for the hardware stage). H.264:
//! Annex B access units in, NV12 out (the first NV12 type the
//! decoder offers), converted to BGRA with `nv12_to_bgra` (BT.709) or, after
//! `prefer_yuv(true)` (SCR-M7, the viewer's GPU path), kept as planes without
//! the conversion. The decoder's buffer may be taller than the picture (rows padded to 16); its
//! row pitch follows from the buffer length, and only the picture's area is
//! converted. A changed stream (`MF_E_TRANSFORM_STREAM_CHANGE`) selects NV12
//! again.
//!
//! Stages (`bildkette.md` BK5w, rules in `mf_decoder_stage.rs`): with a
//! D3D11 device manager (DXVA) when the device decodes the stream's profile
//! and size, NV12 textures read back into the same planes
//! (`mf_decoder_d3d.rs`); otherwise the same transform in software. A
//! hardware failure while decoding ends with an error; the decoder opened
//! for the requested key frame starts at the next stage. Low latency:
//! `MF_LOW_LATENCY` and `CODECAPI_AVLowLatencyMode` (the same GUID); no
//! reordering beyond the stream's own (se hosts send no B frames).
use std::io;
use std::sync::Arc;

use windows::core::{Interface, VARIANT};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVLowLatencyMode, ICodecAPI, IMFSample, IMFTransform, MFVideoFormat_NV12,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MF_E_NOTACCEPTING,
    MF_LOW_LATENCY, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE,
};

use super::mf_common::{activate_codec, ensure_com, Kind};
use super::mf_encoder_sync::{stop, UNITS_PER_SECOND};
use super::mf_media::{input_sample, process_output_bounded, set_value, video_type, Output};
use crate::screen::core::decoded_frame::{DecodedFrame, DecoderFallback, FrameIdentities};
use crate::screen::core::host_backend::{Chroma, ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::{chosen_decoder_preference, CodecKind};
use crate::screen::core::pixel_convert::{nv12_to_bgra, BgraImage, FrameSize, Nv12View, YuvMatrix};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

#[path = "mf_decoder_d3d.rs"]
mod d3d;
#[path = "mf_decoder_planes.rs"]
mod planes;
#[path = "mf_decoder_stage.rs"]
mod stage;
pub(super) use planes::keeps_references;
pub(super) use stage::DecoderPreference;
use stage::{known, remember, DecodeStage, HardwareFailure};

/// Time stamps of the input samples: 30 frames per second.
const FRAME_UNITS: i64 = UNITS_PER_SECOND / 30;
/// Output types the decoder may list before NV12.
const MAX_TYPES: u32 = 64;
/// Most rows or columns a decoder's NV12 buffer may add to the picture:
/// one HEVC coding tree block (64) or AV1 superblock (128) less one (plan
/// 2026-11-17 P3: 15 refused every HEVC and AV1 stream whose decoder aligns
/// to its block size).
const MAX_PADDING: u32 = 127;

/// Whether this host decodes `codec` in hardware (`se screen codec`).
pub(super) fn hardware_offered(codec: CodecKind) -> bool {
    d3d::hardware_offered(codec)
}

pub(super) struct MfDecoder {
    transform: IMFTransform,
    /// H.264, HEVC or AV1 (BK6).
    codec: CodecKind,
    /// The transform's name and the stage it runs on.
    name: String,
    stage: DecodeStage,
    /// The read-back of the hardware stage.
    readback: Option<d3d::TextureReadback>,
    /// A hardware failure the decoder recognised itself.
    noted: Option<HardwareFailure>,
    /// The hardware stage ended; every further record is refused.
    failed: Option<HardwareFailure>,
    /// This decoder delivered a picture.
    produced: bool,
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
    /// Plane storage kept between pictures: read-back target of the
    /// hardware stage, copy target of the software stage, a returned one.
    texture_planes: (Vec<u8>, Vec<u8>),
    /// A shown BGRA picture given back (`recycle_picture`).
    spare_bgra: Option<BgraImage>,
}

// SAFETY: one thread at a time (the decoder is borrowed mutably); the
// transform is a free-threaded object of the multithreaded apartment, the
// device of the read-back is free-threaded (`mf_decoder_d3d`).
unsafe impl Send for MfDecoder {}

impl MfDecoder {
    /// Opens the decoder `index` (`name`) of the enumeration for the group
    /// of `header` at the first stage of `preference` that opens (4.10a).
    pub(super) fn open(
        index: usize,
        name: &str,
        header: &GopHeader,
        preference: DecoderPreference,
    ) -> io::Result<Self> {
        ensure_com();
        let size = (header.size.width, header.size.height);
        let memory = known(header.codec);
        // A chosen „Dekodierer: Hardware“ is never left without its stage
        // (user rule 2026-11-18): when the memory leaves none, hardware is
        // tried again. Under „Automatisch“ the register's software pass
        // follows instead (`chosen_decoder_preference`, not the pass).
        let memory = if chosen_decoder_preference() == DecoderPreference::Hardware
            && stage::mf_stages(preference, true, &memory, size).is_empty()
        {
            stage::StageMemory::new()
        } else {
            memory
        };
        let wanted = stage::ranking(preference).contains(&DecodeStage::Hardware)
            && memory.allows_hardware(size)
            && header.codec.is_video();
        let device = wanted
            .then(d3d::shared)
            .and_then(Result::ok)
            .filter(|device| device.fits(header.codec, header.size));
        let mut refusals: Vec<String> = Vec::new();
        for stage in stage::mf_stages(preference, device.is_some(), &memory, size) {
            let transform = activate_codec(Kind::Decoder, header.codec, index, name)?;
            let hardware = device.clone().filter(|_| stage.hardware());
            match Self::new(transform, name, header, stage, hardware) {
                Ok(decoder) => return Ok(decoder),
                Err(error) => {
                    if stage.hardware() {
                        remember(header.codec, HardwareFailure::OpenRefused, size);
                    }
                    refusals.push(format!("{}: {error}", stage.key()));
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            if refusals.is_empty() {
                format!("{name}: no decoder stage for {preference:?}")
            } else {
                format!("{name}: {}", refusals.join("; "))
            },
        ))
    }

    fn new(
        transform: IMFTransform,
        name: &str,
        header: &GopHeader,
        stage: DecodeStage,
        device: Option<Arc<d3d::DecodeDevice>>,
    ) -> io::Result<Self> {
        let limits = header.frame_contract.map_or(
            crate::screen::core::frame_budget::FrameLimits::LEGACY,
            |c| c.limits,
        );
        limits.check(header.size)?;
        let subtype = super::mf_common::subtype(header.codec)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "MF decodes no tiles"))?;
        let input = video_type(&subtype, header.size, 30)?;
        // The device manager comes before the media types (DXVA decoding).
        if let Some(device) = &device {
            device.attach(&transform)?;
        }
        // SAFETY: attribute writes and type selection on the activated
        // transform; low latency is optional.
        unsafe {
            if let Ok(attributes) = transform.GetAttributes() {
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            if let Ok(api) = transform.cast::<ICodecAPI>() {
                set_value(&api, &CODECAPI_AVLowLatencyMode, VARIANT::from(true));
            }
            transform.SetInputType(0, &input, 0)?;
        }
        let mut decoder = Self {
            transform,
            codec: header.codec,
            name: stage::stage_name(name, stage),
            stage,
            readback: device.map(d3d::TextureReadback::new),
            noted: None,
            failed: None,
            produced: false,
            size: header.size,
            allocated: header.size,
            picture: BgraImage::new_with_limits(header.size, limits)?,
            limits,
            frames: 0,
            identities: FrameIdentities::default(),
            decoded: None,
            yuv: false,
            planes: None,
            texture_planes: (Vec::new(), Vec::new()),
            spare_bgra: None,
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
                || allocated.width > self.size.width.saturating_add(MAX_PADDING)
                || allocated.height > self.size.height.saturating_add(MAX_PADDING)
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

    /// Converts one NV12 output sample into the picture: a decoder texture
    /// through the read-back, a memory buffer directly.
    fn convert(&mut self, sample: &IMFSample) -> io::Result<()> {
        let max = self.limits.check(self.allocated)?;
        if self.readback.is_some() && d3d::is_texture(sample) {
            return self.convert_texture(sample, max);
        }
        // V1: straight from the locked buffer into kept planes.
        self.convert_memory(sample, max)
    }

    /// The hardware stage: reads the decoder's texture back, straight into
    /// the output planes (BK9: no second copy; the window's wgpu device
    /// cannot import the texture, `G/screen_present.rs`).
    fn convert_texture(&mut self, sample: &IMFSample, max: usize) -> io::Result<()> {
        let (mut luma, mut chroma) = self.buffers();
        let read = match self.readback.as_mut() {
            Some(readback) => readback.read(sample, self.size, max, &mut luma, &mut chroma),
            None => Err(io::Error::other("no read-back for a decoder texture")),
        };
        match read {
            Ok(pitch) => self.keep(luma, chroma, pitch),
            Err(error) => {
                self.noted = self.noted.or(Some(HardwareFailure::OutputUnusable));
                self.texture_planes = (luma, chroma);
                Err(error)
            }
        }
    }

    /// Keeps owned NV12 planes with a common row `pitch` as the output
    /// without copying them, or converts them to BGRA.
    fn keep(&mut self, luma: Vec<u8>, chroma: Vec<u8>, pitch: usize) -> io::Result<()> {
        let view = Nv12View::new_with_limits(self.size, &luma, pitch, &chroma, pitch, self.limits);
        let result = match view {
            Ok(view) if !self.yuv => nv12_to_bgra(&view, YuvMatrix::Bt709, &mut self.picture),
            other => other.map(|_| ()),
        };
        if self.yuv && result.is_ok() {
            self.planes = Some(YuvPicture {
                y: luma,
                y_stride: pitch,
                uv: chroma,
                uv_stride: pitch,
                chroma: Chroma::Nv12,
                matrix: YuvMatrix::Bt709,
            });
        } else {
            self.texture_planes = (luma, chroma);
        }
        result
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
                    self.produced = true;
                    shown = true;
                }
                Output::NeedInput => return Ok(shown),
                Output::StreamChange => {
                    if let Err(error) = self.select_nv12() {
                        if self.stage.hardware() {
                            self.noted = self.noted.or(Some(HardwareFailure::FormatUnsupported));
                        }
                        return Err(error);
                    }
                }
            }
        }
    }

    /// Ends the hardware stage when `error` is the hardware's.
    fn on_error(&mut self, error: &io::Error) {
        let Some(readback) = &self.readback else {
            return;
        };
        // A hardware decoder that fails before its first picture is not
        // used again (fix of 2026-11-17): an error it does not classify
        // (E_FAIL, a rejected output) would otherwise repeat at every key
        // frame and leave the window black; the next key frame opens the
        // software stage.
        let noted = self
            .noted
            .take()
            .or_else(|| (!self.produced).then_some(HardwareFailure::OutputUnusable));
        let failure = stage::classify(d3d::hresult_of(error), readback.device_lost(), noted);
        if let Some(failure) = failure {
            remember(self.codec, failure, (self.size.width, self.size.height));
            self.failed = Some(failure);
        }
    }

    fn decode_record(&mut self, record: &FrameRecord) -> io::Result<bool> {
        crate::screen::core::frame_size_caps::check_video_dimensions(
            self.codec,
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
}

impl ScreenDecoder for MfDecoder {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        ensure_com();
        self.decoded = None;
        if let Some(failure) = self.failed {
            return Err(io::Error::other(format!(
                "hardware decoding ended ({}); the next key frame opens the next stage",
                failure.key()
            )));
        }
        let result = self.decode_record(record);
        if let Err(error) = &result {
            self.on_error(error);
        }
        result
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

    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.hand_over_planes()
    }

    fn take_picture(&mut self) -> BgraImage {
        self.hand_over_picture()
    }

    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.take_back(planes);
    }

    fn recycle_picture(&mut self, image: BgraImage) {
        self.take_back_picture(image);
    }

    fn keeps_references(&self) -> bool {
        keeps_references(self.codec)
    }
}

impl Drop for MfDecoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}

#[cfg(test)]
#[path = "mf_decoder_tests.rs"]
mod tests;
