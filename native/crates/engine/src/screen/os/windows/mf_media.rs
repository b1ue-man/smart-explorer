//! Media types, samples and codec settings of the Media Foundation
//! transforms (SCR-M3): progressive NV12 and H.264 types, NV12 input samples
//! from BGRA (BT.709, limited range), the bytes of output samples and their
//! clean-point flag, the encoder settings of decision C2 (low latency, no B
//! frames, CBR) and one `ProcessOutput` with its three outcomes.
//!
//! Plan 2026-11-14 bildkette B-21 (package BK2a), hardware encoders: CBR
//! with a buffer of one frame (bit rate / frame rate; since 2026-11-17 in
//! bytes, as `CODECAPI_AVEncCommonBufferSize` takes it for H.264 and HEVC,
//! before in bits: a buffer of eight frames), no B frames, low
//! latency, the longest GOP the encoder takes (key frames only on request),
//! LTR buffer control where `CODECAPI_AVEncVideoLTRBufferControl` is taken,
//! key frames marked as LTR 0. A value the MFT refuses stays at its default
//! and is listed (`HardwareSettings::refused`). The bit rate is bounded by
//! what the opened encoder and its H.264 level report, not by 240 Mbit/s.
//! B-29 (BK3b): the low-power mode is switched off where the MFT offers it
//! (`CODECAPI_AVEncLowPowerEncoder` = 0), the vendor's full-power state.
//! Plan 2026-11-17 P9: the fastest setting of the quality/speed trade-off
//! (`CODECAPI_AVEncCommonQualityVsSpeed` = 0, "0 = lower quality, faster
//! encoding"), the encoder's fastest preset for the encode time of one frame.
use std::io;
use std::mem::ManuallyDrop;

use windows::core::{Interface, GUID, VARIANT};
use windows::Win32::Media::MediaFoundation::{
    eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_ConstrainedBase, eAVEncH264VProfile_High,
    CODECAPI_AVEncCommonBufferSize, CODECAPI_AVEncCommonMeanBitRate,
    CODECAPI_AVEncCommonQualityVsSpeed, CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncLowPowerEncoder, CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize,
    CODECAPI_AVEncVideoLTRBufferControl, CODECAPI_AVEncVideoMarkLTRFrame,
    CODECAPI_AVLowLatencyMode, ICodecAPI, IMFMediaBuffer, IMFMediaType, IMFSample, IMFTransform,
    MFMediaType_Video, MFSampleExtension_CleanPoint, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE,
    MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
};

use super::mf_common::{platform, EncoderSetup};
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::pixel_convert::{
    bgra_to_i420_with_limits, BgraImage, FrameSize, YuvMatrix,
};

// The bit rate of the encoders (plan 2026-11-17: buffer in bytes, mean bit
// rate read back) lives in `mf_rate.rs`; every `mf_media::` path stays.
#[path = "mf_rate.rs"]
mod rate;
#[cfg(test)]
pub(in crate::screen::os) use rate::{bitrate_limit, level_max_bitrate};
pub(in crate::screen::os) use rate::{
    bounded_bitrate, one_frame_buffer, opened_limit, refused_lowering, retune_bounded,
    set_mean_bitrate, sharp_mode,
};

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// A new, empty media type.
fn media_type() -> io::Result<IMFMediaType> {
    let platform = platform()?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: MFCreateMediaType writes one new reference to `raw`.
    unsafe { (platform.create_media_type)(&mut raw) }.ok()?;
    // SAFETY: the reference written above is owned from here on.
    Ok(unsafe { IMFMediaType::from_raw(raw) })
}

/// A progressive video type of `subtype`, `size` and `fps`.
pub(super) fn video_type(subtype: &GUID, size: FrameSize, fps: u32) -> io::Result<IMFMediaType> {
    crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(size)?;
    let media = media_type()?;
    // SAFETY: plain attribute writes on an owned media type.
    unsafe {
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        media.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.width, size.height))?;
        media.SetUINT64(&MF_MT_FRAME_RATE, pack(fps.max(1), 1))?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    }
    Ok(media)
}

/// The NV12 input type of an encoder (tight rows).
pub(super) fn nv12_type(setup: &EncoderSetup) -> io::Result<IMFMediaType> {
    let media = video_type(&MFVideoFormat_NV12, setup.size, setup.fps)?;
    // SAFETY: attribute write on an owned media type.
    unsafe { media.SetUINT32(&MF_MT_DEFAULT_STRIDE, setup.size.width)? };
    Ok(media)
}

/// The H.264 output type of an encoder with its profile and bit rate.
pub(super) fn h264_type(setup: &EncoderSetup, bitrate: u32) -> io::Result<IMFMediaType> {
    let media = video_type(&MFVideoFormat_H264, setup.size, setup.fps)?;
    let profile = match setup.profile {
        H264Profile::ConstrainedBaseline => eAVEncH264VProfile_ConstrainedBase,
        H264Profile::High => eAVEncH264VProfile_High,
    };
    // SAFETY: attribute writes on an owned media type.
    unsafe {
        media.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        media.SetUINT32(&MF_MT_MPEG2_PROFILE, profile.0 as u32)?;
    }
    Ok(media)
}

/// NV12 bytes (tight) of `frame` in BT.709 limited range.
pub(super) fn nv12_bytes(frame: &BgraImage) -> io::Result<Vec<u8>> {
    let yuv = bgra_to_i420_with_limits(
        &frame.view(),
        YuvMatrix::Bt709,
        crate::screen::core::frame_budget::FrameLimits::RESOURCE,
    )?;
    let mut out = Vec::with_capacity(yuv.y.len() + yuv.u.len() * 2);
    out.extend_from_slice(&yuv.y);
    for (u, v) in yuv.u.iter().zip(&yuv.v) {
        out.push(*u);
        out.push(*v);
    }
    Ok(out)
}

/// A sample with one buffer of `len` bytes, filled by `fill` when given.
fn sample_with(len: usize, fill: Option<&[u8]>) -> io::Result<IMFSample> {
    if len > 64 * 1024 * 1024 {
        return Err(io::Error::other(
            "MF sample exceeds a resource-bounded working image",
        ));
    }
    let platform = platform()?;
    let size = u32::try_from(len.max(1))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "sample is too large"))?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: MFCreateMemoryBuffer writes one new reference to `raw`.
    unsafe { (platform.create_memory_buffer)(size, &mut raw) }.ok()?;
    // SAFETY: the reference written above is owned from here on.
    let buffer = unsafe { IMFMediaBuffer::from_raw(raw) };
    if let Some(bytes) = fill {
        let mut data: *mut u8 = std::ptr::null_mut();
        // SAFETY: the locked buffer holds at least `size` >= `bytes.len()`
        // bytes until `Unlock`.
        unsafe {
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes.len() as u32)?;
        }
    }
    let mut raw = std::ptr::null_mut();
    // SAFETY: MFCreateSample writes one new reference to `raw`.
    unsafe { (platform.create_sample)(&mut raw) }.ok()?;
    // SAFETY: the reference written above is owned from here on.
    let sample = unsafe { IMFSample::from_raw(raw) };
    // SAFETY: the sample takes its own reference to the buffer.
    unsafe { sample.AddBuffer(&buffer)? };
    Ok(sample)
}

/// An input sample with `bytes` at `time` (100 ns units) for `duration`.
pub(super) fn input_sample(bytes: &[u8], time: i64, duration: i64) -> io::Result<IMFSample> {
    let sample = sample_with(bytes.len(), Some(bytes))?;
    // SAFETY: plain attribute writes on an owned sample.
    unsafe {
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(duration)?;
    }
    Ok(sample)
}

/// The bytes of `sample` in one piece.
pub(super) fn sample_bytes(sample: &IMFSample) -> io::Result<Vec<u8>> {
    sample_bytes_bounded(sample, crate::screen::core::wire_binary::MAX_RECORD_BYTES)
}
pub(super) fn sample_bytes_bounded(sample: &IMFSample, max: usize) -> io::Result<Vec<u8>> {
    // Check before MF coalesces the buffers, which can allocate a second full sample.
    if unsafe { sample.GetTotalLength()? } as usize > max {
        return Err(io::Error::other("MF output exceeds admitted sample bytes"));
    }
    // SAFETY: the buffer stays locked only while its bytes are copied.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer()?;
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut length = 0u32;
        buffer.Lock(&mut data, None, Some(std::ptr::addr_of_mut!(length)))?;
        if length as usize > max {
            buffer.Unlock()?;
            return Err(io::Error::other(
                "MF locked output exceeds admitted sample bytes",
            ));
        }
        let bytes = if data.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data, length as usize).to_vec()
        };
        buffer.Unlock()?;
        Ok(bytes)
    }
}

/// Whether an encoded sample is a key frame (clean point).
pub(super) fn is_keyframe(sample: &IMFSample) -> bool {
    // SAFETY: attribute read on an owned sample.
    unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }.is_ok_and(|value| value != 0)
}

/// Sets `key` to `value`; `false` when the encoder refuses it.
pub(super) fn set_value(api: &ICodecAPI, key: &GUID, value: VARIANT) -> bool {
    // SAFETY: `value` outlives the call, which copies it.
    unsafe { api.SetValue(key, &value) }.is_ok()
}

/// Low latency, no B frames, CBR at `bitrate` (C2 „Niedriglatenz“).
pub(super) fn configure_encoder(transform: &IMFTransform, bitrate: u32) -> Option<ICodecAPI> {
    let api = transform.cast::<ICodecAPI>().ok()?;
    let cbr = eAVEncCommonRateControlMode_CBR.0 as u32;
    set_value(&api, &CODECAPI_AVLowLatencyMode, VARIANT::from(true));
    set_value(
        &api,
        &CODECAPI_AVEncMPVDefaultBPictureCount,
        VARIANT::from(0u32),
    );
    set_value(
        &api,
        &CODECAPI_AVEncCommonRateControlMode,
        VARIANT::from(cbr),
    );
    set_value(
        &api,
        &CODECAPI_AVEncCommonMeanBitRate,
        VARIANT::from(bitrate),
    );
    Some(api)
}

/// What one `ProcessOutput` gave.
pub(super) enum Output {
    Sample(IMFSample),
    NeedInput,
    StreamChange,
}

/// One `ProcessOutput` on stream 0; a transform that does not provide its
/// samples gets one of its announced size, at least `fallback` bytes.
pub(super) fn process_output(transform: &IMFTransform, fallback: usize) -> io::Result<Output> {
    process_output_bounded(transform, fallback, 64 * 1024 * 1024)
}
pub(super) fn process_output_bounded(
    transform: &IMFTransform,
    fallback: usize,
    max: usize,
) -> io::Result<Output> {
    // SAFETY: a plain query of the output stream.
    let info = unsafe { transform.GetOutputStreamInfo(0)? };
    if fallback > max || info.cbSize as usize > max {
        return Err(io::Error::other(
            "MF output buffer exceeds admitted image bytes",
        ));
    }
    let provides =
        (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
    let own = if info.dwFlags & provides != 0 {
        None
    } else {
        Some(sample_with((info.cbSize as usize).max(fallback), None)?)
    };
    let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
        dwStreamID: 0,
        pSample: ManuallyDrop::new(own),
        dwStatus: 0,
        pEvents: ManuallyDrop::new(None),
    }];
    let mut status = 0u32;
    // SAFETY: one buffer for stream 0; its references are taken back below.
    let result = unsafe { transform.ProcessOutput(0, &mut buffers, &mut status) };
    // SAFETY: every reference the call left in the buffer is taken out once.
    let (sample, events) = unsafe {
        (
            ManuallyDrop::take(&mut buffers[0].pSample),
            ManuallyDrop::take(&mut buffers[0].pEvents),
        )
    };
    drop(events);
    match result {
        Ok(()) => sample
            .map(Output::Sample)
            .ok_or_else(|| io::Error::other("Media Foundation gave no output sample")),
        Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(Output::NeedInput),
        Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => Ok(Output::StreamChange),
        Err(error) => Err(io::Error::other(error)),
    }
}

/// One setting of B-21 and whether the encoder took it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::screen::os) struct Setting {
    pub name: &'static str,
    pub accepted: bool,
}

/// What a hardware encoder took of B-21.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::screen::os) struct HardwareSettings {
    pub settings: Vec<Setting>,
    /// Long-term reference frames under the application's control (0: none).
    pub ltr_frames: u16,
}

impl HardwareSettings {
    fn note(&mut self, name: &'static str, accepted: bool) {
        self.settings.push(Setting { name, accepted });
    }
    /// The settings the encoder refused; they stay at its default.
    pub(in crate::screen::os) fn refused(&self) -> Vec<&'static str> {
        self.settings
            .iter()
            .filter(|setting| !setting.accepted)
            .map(|setting| setting.name)
            .collect()
    }
}

/// Names of the B-21 settings in the order they are applied.
pub(in crate::screen::os) const SETTINGS: [&str; 9] = [
    "low_latency",
    "b_frames_0",
    "cbr",
    "mean_bitrate",
    "buffer_one_frame",
    "gop_infinite",
    "ltr",
    "low_power_off",
    "quality_speed",
];

/// `CODECAPI_AVEncVideoLTRBufferControl`: trust mode 1 in the upper 16
/// bits, the number of LTR frames in the lower 16.
pub(in crate::screen::os) fn ltr_control(frames: u16) -> u32 {
    (1 << 16) | u32::from(frames)
}

/// GOP lengths to try, longest first: the encoder's own maximum, then the
/// largest values a `u32`, `u16` and `i16` field holds (0 is not used: its
/// meaning differs between encoders).
pub(in crate::screen::os) fn gop_candidates(encoder_max: Option<u32>) -> Vec<u32> {
    let mut list: Vec<u32> = encoder_max.filter(|max| *max > 0).into_iter().collect();
    for value in [u32::MAX, u32::from(u16::MAX), i16::MAX as u32] {
        if !list.contains(&value) {
            list.push(value);
        }
    }
    list
}

/// The largest value the encoder reports for `key`.
pub(super) fn range_max(api: &ICodecAPI, key: &GUID) -> Option<u32> {
    let mut low = VARIANT::default();
    let mut high = VARIANT::default();
    let mut step = VARIANT::default();
    // SAFETY: the three VARIANTs are owned locals the call fills.
    unsafe { api.GetParameterRange(key, &mut low, &mut high, &mut step) }.ok()?;
    u32::try_from(&high).ok()
}

/// B-21 on a hardware encoder, before its media types are set.
pub(in crate::screen::os) fn configure_hardware(
    transform: &IMFTransform,
    bitrate: u32,
    fps: u32,
) -> (Option<ICodecAPI>, HardwareSettings) {
    let mut taken = HardwareSettings::default();
    let Ok(api) = transform.cast::<ICodecAPI>() else {
        for name in SETTINGS {
            taken.note(name, false);
        }
        return (None, taken);
    };
    let cbr = eAVEncCommonRateControlMode_CBR.0 as u32;
    let set = |key: &GUID, value: VARIANT| set_value(&api, key, value);
    let low_latency = set(&CODECAPI_AVLowLatencyMode, VARIANT::from(true));
    taken.note(SETTINGS[0], low_latency);
    let b_frames = set(&CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0u32));
    taken.note(SETTINGS[1], b_frames);
    let constant = set(&CODECAPI_AVEncCommonRateControlMode, VARIANT::from(cbr));
    taken.note(SETTINGS[2], constant);
    // Read back (plan 2026-11-17): a rate the MFT does not keep is refused.
    taken.note(SETTINGS[3], !set_mean_bitrate(&api, bitrate));
    let buffer = VARIANT::from(one_frame_buffer(bitrate, fps));
    taken.note(SETTINGS[4], set(&CODECAPI_AVEncCommonBufferSize, buffer));
    let gop = gop_candidates(range_max(&api, &CODECAPI_AVEncMPVGOPSize))
        .into_iter()
        .any(|length| set(&CODECAPI_AVEncMPVGOPSize, VARIANT::from(length)));
    taken.note(SETTINGS[5], gop);
    let ltr = [2u16, 1].into_iter().find(|frames| {
        let control = VARIANT::from(ltr_control(*frames));
        set(&CODECAPI_AVEncVideoLTRBufferControl, control)
    });
    taken.note(SETTINGS[6], ltr.is_some());
    taken.ltr_frames = ltr.unwrap_or(0);
    // B-29: full power instead of the low-power mode [assumption: 0 = off].
    let full_power = set(&CODECAPI_AVEncLowPowerEncoder, VARIANT::from(0u32));
    taken.note(SETTINGS[7], full_power);
    let speed = set(&CODECAPI_AVEncCommonQualityVsSpeed, VARIANT::from(0u32));
    taken.note(SETTINGS[8], speed);
    (Some(api), taken)
}

/// Marks the next frame (a key frame) as long-term reference 0.
pub(in crate::screen::os) fn mark_ltr(api: Option<&ICodecAPI>) {
    if let Some(api) = api {
        set_value(api, &CODECAPI_AVEncVideoMarkLTRFrame, VARIANT::from(0u32));
    }
}
