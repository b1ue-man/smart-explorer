//! The software H.264 encoder of Media Foundation (`Mfh264enc.dll`, a
//! synchronous MFT; SCR-M3, decision C2): NV12 in system memory in, one
//! Annex B access unit out for each frame in low-latency mode. The bit rate
//! changes at run time (`CODECAPI_AVEncCommonMeanBitRate`), a key frame
//! comes on request (`CODECAPI_AVEncVideoForceKeyFrame`). A frame that does
//! not come out of the same call, or a requested key frame that is none, is
//! an encoder failure.
//!
//! Plan 2026-11-14 bildkette (BK3b): the bit rate is bounded by what the
//! opened encoder and its level report (`mf_media::opened_limit`, the rule
//! of B-06/B-21), no longer by a fixed 240 Mbit/s; a picture that lay on the
//! GPU comes in as its NV12 copied from the ring slot. Plan 2026-11-17 P3:
//! key frames follow rule D8 (`mf_capture_output.rs`).
use std::io;

use windows::core::VARIANT;
use windows::Win32::Media::MediaFoundation::{
    eAVEncAV1VProfile_Main_420_8, eAVEncH265VProfile_Main_420_8, CODECAPI_AVEncVideoForceKeyFrame,
    ICodecAPI, IMFMediaType, IMFTransform, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_END_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MF_MT_AVG_BITRATE, MF_MT_MPEG2_PROFILE,
    MF_TRANSFORM_ASYNC_UNLOCK,
};

use super::mf_common::{ensure_com, subtype, EncoderSetup};
use super::mf_media::{
    bounded_bitrate, configure_encoder, configure_hardware, floored, h264_type, input_sample,
    is_keyframe, nv12_bytes, nv12_type, opened_limit, process_output, refused_lowering,
    sample_bytes_bounded, set_mean_bitrate, set_value, video_type, Output,
};
use super::{mf_capture_media, mf_capture_output};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::{AdmittedBytes, CapturedImage};
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::rate_control::honour::EncoderRateState;
use crate::screen::core::rate_control::START_DIRECT_BPS;

/// Media Foundation time units (100 ns) per second.
pub(super) const UNITS_PER_SECOND: i64 = 10_000_000;

/// Sets a changed bit rate (at least 2 % off) within `limit`, the ceiling
/// the opened encoder and its level report; a refusal keeps the old one.
/// `None` when nothing was set, else whether the encoder refused the rate
/// or reads back another one (`set_mean_bitrate`, plan 2026-11-17).
pub(super) fn retune(
    api: Option<&ICodecAPI>,
    current: &mut u32,
    target_bps: u64,
    limit: u32,
) -> Option<bool> {
    let target = bounded_bitrate(target_bps, limit);
    // Review 2026-11-19 L1: the 2 % band holds back rises only; every
    // lowering is set, so the encoder never stays above a lowered rate.
    if target >= *current && (i64::from(target) - i64::from(*current)) * 50 < i64::from(*current) {
        return None;
    }
    // Codex 14 (review 2026-11-19): without `ICodecAPI` a lowering cannot
    // be set; it is a refused one, not nothing to do.
    let Some(api) = api else {
        return (target < *current).then_some(true);
    };
    let refused = set_mean_bitrate(api, target);
    if !refused {
        *current = target;
    }
    Some(refused)
}

/// Asks for an IDR as the next frame.
pub(super) fn force_keyframe(api: Option<&ICodecAPI>) {
    if let Some(api) = api {
        set_value(api, &CODECAPI_AVEncVideoForceKeyFrame, VARIANT::from(1u32));
    }
}

/// The output type of `setup`: H.264 with its profile, HEVC Main or AV1
/// Main, both 8-bit 4:2:0 (`EncoderSetup::codec`, B-23, merge W1; the
/// profile constants as the crate `windows` 0.58.0 binds them:
/// `eAVEncH265VProfile_Main_420_8` = 1, `eAVEncAV1VProfile_Main_420_8` = 1).
pub(super) fn output_type(setup: &EncoderSetup, bitrate: u32) -> io::Result<IMFMediaType> {
    let profile = match setup.codec {
        CodecKind::H264 => return h264_type(setup, bitrate),
        CodecKind::Hevc => eAVEncH265VProfile_Main_420_8.0 as u32,
        CodecKind::Av1 => eAVEncAV1VProfile_Main_420_8.0 as u32,
        CodecKind::Tiles => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Media Foundation encodes no tiles",
            ))
        }
    };
    let compressed = subtype(setup.codec)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no MF subtype"))?;
    let media = video_type(&compressed, setup.size, setup.fps)?;
    // SAFETY: attribute writes on an owned media type.
    unsafe {
        media.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        media.SetUINT32(&MF_MT_MPEG2_PROFILE, profile)?;
    }
    Ok(media)
}

/// The B-21 settings a hardware MFT leaves at its default (`se screen codec
/// --json`, merge W1): unlocked and configured as `AsyncEncoder::create`
/// does before its media types are set; nothing streams.
pub(super) fn probe_refused(transform: &IMFTransform) -> Vec<String> {
    // SAFETY: an attribute write on the activated transform; a transform
    // that is not asynchronous ignores it.
    let _ = unsafe {
        transform
            .GetAttributes()
            .and_then(|attributes| attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1))
    };
    let bitrate = bounded_bitrate(START_DIRECT_BPS, u32::MAX);
    let (_, settings) = configure_hardware(transform, bitrate, 60);
    settings.refused().into_iter().map(String::from).collect()
}

/// Opens `transform` for `setup`: output type first, then input type (the
/// order encoders require), then streaming starts.
pub(super) fn start(
    transform: &IMFTransform,
    setup: &EncoderSetup,
    bitrate: u32,
) -> io::Result<()> {
    let output = output_type(setup, bitrate)?;
    let input = nv12_type(setup)?;
    // SAFETY: plain calls on an activated transform with owned types.
    unsafe {
        transform.SetOutputType(0, &output, 0)?;
        transform.SetInputType(0, &input, 0)?;
        transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
    }
    Ok(())
}

/// Ends streaming of `transform` (drop of an encoder).
pub(super) fn stop(transform: &IMFTransform) {
    // SAFETY: notifications only; errors do not matter at the end.
    unsafe {
        let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
        let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
    }
}

pub(in crate::screen::os) fn update_refresh<T>(refresh: &mut bool, result: &io::Result<T>) {
    match result {
        Ok(_) => *refresh = false,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => *refresh = true,
        Err(_) => {}
    }
}

pub(super) struct SyncEncoder {
    transform: IMFTransform,
    api: Option<ICodecAPI>,
    setup: EncoderSetup,
    frames: i64,
    bitrate: u32,
    /// Bit rate ceiling of the opened encoder and its level.
    limit: u32,
    grant: Option<CaptureGrant>,
    /// A frame was skipped (`WouldBlock`): the next request asks for a picture.
    refresh: bool,
    /// The MFT refused its mean bit rate or reads back another one (plan
    /// 2026-11-17, `ScreenEncoder::rate_state`).
    rate_refused: bool,
    /// D8: the last parameter sets and those of the output type.
    parameter_sets: Option<Vec<u8>>,
    sequence_header: Option<Vec<u8>>,
}

// SAFETY: the transform is used by one thread at a time (the encoder is
// borrowed mutably); Media Foundation transforms are free-threaded objects
// of the multithreaded apartment.
unsafe impl Send for SyncEncoder {}

impl SyncEncoder {
    pub(super) fn new(transform: IMFTransform, setup: EncoderSetup) -> io::Result<Self> {
        Self::create(transform, setup, None, START_DIRECT_BPS)
    }

    /// Opens at `bps`, the rate the session gives it (plan 2026-11-17).
    pub(super) fn new_admitted(
        transform: IMFTransform,
        setup: EncoderSetup,
        grant: CaptureGrant,
        bps: u64,
    ) -> io::Result<Self> {
        Self::create(transform, setup, Some(grant), bps)
    }

    fn create(
        transform: IMFTransform,
        setup: EncoderSetup,
        grant: Option<CaptureGrant>,
        bps: u64,
    ) -> io::Result<Self> {
        let bitrate = bounded_bitrate(bps, u32::MAX);
        let bitrate = super::mf_media::opening_rate(&transform, bitrate); // X2
        let api = configure_encoder(&transform, bitrate);
        start(&transform, &setup, bitrate)?;
        let limit = opened_limit(&transform, api.as_ref(), &setup);
        let sequence_header = mf_capture_output::sequence_header(&transform);
        Ok(Self {
            transform,
            api,
            setup,
            frames: 0,
            bitrate,
            limit,
            grant,
            refresh: false,
            rate_refused: false,
            parameter_sets: None,
            sequence_header,
        })
    }

    fn encode_admitted(
        &mut self,
        image: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| io::Error::other("MF encoder has no capture grant"))?;
        let maximum = mf_capture_output::capture_limit(&grant, image, request, self.setup.size)?;
        ensure_com();
        let api = self.api.as_ref();
        let target_bps = floored(api, request.target_bps, self.bitrate);
        if let Some(refused) = retune(api, &mut self.bitrate, target_bps, self.limit) {
            self.rate_refused = refused;
            if refused {
                // Review 2026-11-19 B6: still above a lowered rate, it gives way.
                refused_lowering(self.bitrate, target_bps, self.limit)?;
            }
        }
        if request.keyframe && self.frames > 0 {
            force_keyframe(self.api.as_ref());
        }
        let duration = UNITS_PER_SECOND / i64::from(self.setup.fps.max(1));
        let time = self
            .frames
            .checked_mul(duration)
            .ok_or_else(|| io::Error::other("MF frame timestamp overflow"))?;
        let sample = mf_capture_media::input_sample(&grant, image, time, duration)?;
        self.frames = self
            .frames
            .checked_add(1)
            .ok_or_else(|| io::Error::other("MF frame count overflow"))?;
        unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        // Rust's input reference may disappear here; a retained MFT sample
        // still owns its buffer wrapper and therefore the input slice.
        drop(sample);
        let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, maximum as u64)?;
        let mut keyframe = false;
        loop {
            match mf_capture_output::process_output(
                &self.transform,
                &grant,
                self.setup.size.bgra_len(),
            )? {
                Output::Sample(sample) => {
                    keyframe |= is_keyframe(&sample);
                    mf_capture_output::append_unit_sample(
                        &mut bytes,
                        &sample,
                        self.setup.codec,
                        &mut self.parameter_sets,
                        self.sequence_header.as_deref(),
                    )?;
                }
                Output::NeedInput => break,
                Output::StreamChange => {
                    return Err(io::Error::other("the encoder changed its output type"))
                }
            }
        }
        mf_capture_output::finish(
            bytes,
            self.setup.codec,
            self.setup.size,
            keyframe,
            request.keyframe,
        )
    }
}

impl ScreenEncoder for SyncEncoder {
    fn encode_captured(
        &mut self,
        image: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        self.encode_picture(&Picture::Cpu(image.clone()), request)
    }

    /// A GPU picture comes in as its NV12 copied from the slot (BK3b).
    fn encode_picture(
        &mut self,
        image: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let result = self.encode_admitted(image, request);
        update_refresh(&mut self.refresh, &result);
        result
    }

    fn name(&self) -> String {
        self.setup.name.clone()
    }

    fn size(&self) -> FrameSize {
        self.setup.size
    }

    fn needs_refresh(&self) -> bool {
        self.refresh
    }

    fn rate_state(&self) -> Option<EncoderRateState> {
        Some(EncoderRateState {
            applied_bps: u64::from(self.bitrate),
            refused: self.rate_refused,
        })
    }

    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        if self.grant.is_some() {
            return Err(io::Error::other(
                "admitted MF encoder requires encode_captured",
            ));
        }
        if frame.size() != self.setup.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the Media Foundation encoder",
            ));
        }
        ensure_com();
        let api = self.api.as_ref();
        let target_bps = floored(api, target_bps, self.bitrate);
        if let Some(refused) = retune(api, &mut self.bitrate, target_bps, self.limit) {
            self.rate_refused = refused;
            if refused {
                // Codex 14: the same contract as `encode_admitted` (B6).
                refused_lowering(self.bitrate, target_bps, self.limit)?;
            }
        }
        if keyframe && self.frames > 0 {
            force_keyframe(self.api.as_ref());
        }
        let duration = UNITS_PER_SECOND / i64::from(self.setup.fps.max(1));
        let sample = input_sample(&nv12_bytes(frame)?, self.frames * duration, duration)?;
        self.frames += 1;
        // SAFETY: one owned input sample for stream 0.
        unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        let fallback = self.setup.size.bgra_len();
        let (mut parts, mut key) = (Vec::new(), false);
        let mut total = 0usize;
        loop {
            match process_output(&self.transform, fallback)? {
                Output::Sample(out) => {
                    key |= is_keyframe(&out);
                    let part = sample_bytes_bounded(
                        &out,
                        crate::screen::core::wire_binary::MAX_RECORD_BYTES - total,
                    )?;
                    total += part.len();
                    parts.push(part);
                }
                Output::NeedInput => break,
                Output::StreamChange => {
                    return Err(io::Error::other("the encoder changed its output type"));
                }
            }
        }
        // D8: the mark against the stream, parameter sets completed.
        let frame = mf_capture_output::checked_unit(
            self.setup.codec,
            parts.concat(),
            key,
            keyframe,
            &mut self.parameter_sets,
            self.sequence_header.as_deref(),
        )?;
        for data in &frame.records {
            crate::screen::core::frame_size_caps::check_video_dimensions(
                self.setup.codec,
                data,
                self.setup.size,
                crate::screen::core::frame_budget::FrameLimits::RESOURCE,
            )?;
        }
        Ok(frame)
    }
}

impl Drop for SyncEncoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}
