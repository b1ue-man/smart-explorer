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
use super::mf_media::{
    configure_encoder, h264_type, input_sample, is_keyframe, nv12_bytes, nv12_type, process_output,
    sample_bytes, set_value, Output,
};
use crate::screen::core::host_backend::{EncodedFrame, ScreenEncoder};
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
    let bytes: Vec<u8> = parts.concat();
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
}

// SAFETY: the transform is used by one thread at a time (the encoder is
// borrowed mutably); Media Foundation transforms are free-threaded objects
// of the multithreaded apartment.
unsafe impl Send for SyncEncoder {}

impl SyncEncoder {
    pub(super) fn new(transform: IMFTransform, setup: EncoderSetup) -> io::Result<Self> {
        let bitrate = clamp_bitrate(START_DIRECT_BPS);
        let api = configure_encoder(&transform, bitrate);
        start(&transform, &setup, bitrate)?;
        Ok(Self {
            transform,
            api,
            setup,
            frames: 0,
            bitrate,
        })
    }
}

impl ScreenEncoder for SyncEncoder {
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
        loop {
            match process_output(&self.transform, fallback)? {
                Output::Sample(out) => {
                    key |= is_keyframe(&out);
                    parts.push(sample_bytes(&out)?);
                }
                Output::NeedInput => break,
                Output::StreamChange => {
                    return Err(io::Error::other("the encoder changed its output type"));
                }
            }
        }
        frame_of(parts, key, keyframe)
    }
}

impl Drop for SyncEncoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}
