//! The software H.264 encoder of Media Foundation (`Mfh264enc.dll`, a
//! synchronous MFT; SCR-M3, decision C2): NV12 in system memory in, one
//! Annex B access unit out for each frame in low-latency mode. The bit rate
//! changes at run time (`CODECAPI_AVEncCommonMeanBitRate`), a key frame
//! comes on request (`CODECAPI_AVEncVideoForceKeyFrame`). A frame that does
//! not come out of the same call, or a requested key frame that is none, is
//! an encoder failure.
use std::io;

use windows::core::VARIANT;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncVideoForceKeyFrame, ICodecAPI, IMFTransform,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM,
    MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
};

use super::mf_common::{ensure_com, EncoderSetup};
use super::{mf_capture_media, mf_capture_output};
use super::mf_media::{
    configure_encoder, h264_type, input_sample, is_keyframe, nv12_bytes, nv12_type, process_output,
    sample_bytes_bounded, set_value, Output,
};
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::{AdmittedBytes, CapturedImage};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::rate_control::{MAX_BITRATE_BPS, MIN_BITRATE_BPS, START_DIRECT_BPS};

/// Media Foundation time units (100 ns) per second.
pub(super) const UNITS_PER_SECOND: i64 = 10_000_000;

pub(super) fn clamp_bitrate(target_bps: u64) -> u32 {
    target_bps.clamp(MIN_BITRATE_BPS, MAX_BITRATE_BPS) as u32
}

/// Sets a changed bit rate (at least 2 % off); a refusal keeps the old one.
pub(super) fn retune(api: Option<&ICodecAPI>, current: &mut u32, target_bps: u64) {
    let target = clamp_bitrate(target_bps);
    if (i64::from(target) - i64::from(*current)).abs() * 50 < i64::from(*current) {
        return;
    }
    let mean = &CODECAPI_AVEncCommonMeanBitRate;
    if api.is_some_and(|api| set_value(api, mean, VARIANT::from(target))) {
        *current = target;
    }
}

/// Asks for an IDR as the next frame.
pub(super) fn force_keyframe(api: Option<&ICodecAPI>) {
    if let Some(api) = api {
        set_value(api, &CODECAPI_AVEncVideoForceKeyFrame, VARIANT::from(1u32));
    }
}

/// Opens `transform` for `setup`: output type first, then input type (the
/// order encoders require), then streaming starts.
pub(super) fn start(
    transform: &IMFTransform,
    setup: &EncoderSetup,
    bitrate: u32,
) -> io::Result<()> {
    let output = h264_type(setup, bitrate)?;
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

/// The records of one encoded frame: one access unit, or an error when the
/// frame is missing or a requested key frame is none.
pub(super) fn frame_of(parts: Vec<Vec<u8>>, key: bool, wanted: bool) -> io::Result<EncodedFrame> {
    let total=parts.iter().try_fold(0usize,|sum,part|sum.checked_add(part.len()))
        .filter(|n|*n<=crate::screen::core::wire_binary::MAX_RECORD_BYTES)
        .ok_or_else(||io::Error::other("MF access unit exceeds 16 MiB"))?;
    let mut bytes=Vec::with_capacity(total);
    for part in parts {bytes.extend_from_slice(&part);}
    if bytes.is_empty() {
        return Err(io::Error::other(
            "Media Foundation gave no picture for the frame",
        ));
    }
    if wanted && !key {
        return Err(io::Error::other(
            "Media Foundation gave no key frame on request",
        ));
    }
    Ok(EncodedFrame {
        records: vec![bytes],
        keyframe: key,
    })
}

pub(super) struct SyncEncoder {
    transform: IMFTransform,
    api: Option<ICodecAPI>,
    setup: EncoderSetup,
    frames: i64,
    bitrate: u32,
    grant: Option<CaptureGrant>,
}

// SAFETY: the transform is used by one thread at a time (the encoder is
// borrowed mutably); Media Foundation transforms are free-threaded objects
// of the multithreaded apartment.
unsafe impl Send for SyncEncoder {}

impl SyncEncoder {
    pub(super) fn new(transform: IMFTransform, setup: EncoderSetup) -> io::Result<Self> {
        Self::create(transform, setup, None)
    }

    pub(super) fn new_admitted(transform: IMFTransform, setup: EncoderSetup,
        grant: CaptureGrant) -> io::Result<Self> {
        Self::create(transform, setup, Some(grant))
    }

    fn create(transform: IMFTransform, setup: EncoderSetup, grant: Option<CaptureGrant>) -> io::Result<Self> {
        let bitrate = clamp_bitrate(START_DIRECT_BPS);
        let api = configure_encoder(&transform, bitrate);
        start(&transform, &setup, bitrate)?;
        Ok(Self {
            transform,
            api,
            setup,
            frames: 0,
            bitrate,
            grant,
        })
    }
}

impl ScreenEncoder for SyncEncoder {
    fn encode_captured(&mut self, image: &CapturedImage, request: &StageRequest)
        -> io::Result<CapturedEncodedFrame> {
        let grant = self.grant.clone().ok_or_else(|| io::Error::other("MF encoder has no capture grant"))?;
        let maximum = mf_capture_output::capture_limit(&grant, image, request, self.setup.size)?;
        ensure_com();
        retune(self.api.as_ref(), &mut self.bitrate, request.target_bps);
        if request.keyframe && self.frames > 0 {force_keyframe(self.api.as_ref());}
        let duration = UNITS_PER_SECOND / i64::from(self.setup.fps.max(1));
        let time = self.frames.checked_mul(duration).ok_or_else(|| io::Error::other("MF frame timestamp overflow"))?;
        let sample = mf_capture_media::input_sample(&grant, image, time, duration)?;
        self.frames = self.frames.checked_add(1).ok_or_else(|| io::Error::other("MF frame count overflow"))?;
        unsafe {self.transform.ProcessInput(0, &sample, 0)?};
        // Rust's input reference may disappear here; a retained MFT sample
        // still owns its buffer wrapper and therefore the input slice.
        drop(sample);
        let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, maximum as u64)?;
        let mut keyframe = false;
        loop {
            match mf_capture_output::process_output(&self.transform, &grant, self.setup.size.bgra_len())? {
                Output::Sample(sample) => {
                    keyframe |= is_keyframe(&sample);
                    mf_capture_output::append_sample(&mut bytes, &sample)?;
                }
                Output::NeedInput => break,
                Output::StreamChange => return Err(io::Error::other("the encoder changed its output type")),
            }
        }
        mf_capture_output::finish(bytes, self.setup.size, keyframe, request.keyframe)
    }

    fn name(&self) -> String {
        self.setup.name.clone()
    }

    fn size(&self) -> FrameSize {
        self.setup.size
    }

    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        if self.grant.is_some() {
            return Err(io::Error::other("admitted MF encoder requires encode_captured"));
        }
        if frame.size() != self.setup.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the Media Foundation encoder",
            ));
        }
        ensure_com();
        retune(self.api.as_ref(), &mut self.bitrate, target_bps);
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
        let mut total=0usize;
        loop {
            match process_output(&self.transform, fallback)? {
                Output::Sample(out) => {
                    key |= is_keyframe(&out);
                    let part=sample_bytes_bounded(&out,crate::screen::core::wire_binary::MAX_RECORD_BYTES-total)?;
                    total+=part.len();parts.push(part);
                }
                Output::NeedInput => break,
                Output::StreamChange => {
                    return Err(io::Error::other("the encoder changed its output type"));
                }
            }
        }
        let frame=frame_of(parts,key,keyframe)?;
        for data in &frame.records {crate::screen::core::frame_size_caps::check_avc_dimensions(data,self.setup.size,crate::screen::core::frame_budget::FrameLimits::RESOURCE)?;}
        Ok(frame)
    }
}

impl Drop for SyncEncoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}
