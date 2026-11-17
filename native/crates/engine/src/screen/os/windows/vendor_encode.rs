//! The encoder hull around one vendor session (BK12, B-22: NVENC and AMF on
//! the capture's D3D11 device). What the hull owns:
//!
//! - Input: the pipeline's `capture_picture::Picture` (BK3b; merge W1: the
//!   hull implements `encode_picture`) as the texture from
//!   `GpuInput::texture` (BK2a): the NV12 ring slot the video processor
//!   wrote before the capture released its frame, scaled on the GPU where
//!   the sizes differ, or a CPU picture converted on all cores and copied
//!   up. No CPU copy of a GPU picture.
//! - Codecs: H.264, and HEVC/AV1 (merge W1, B-23); a HEVC or AV1 key frame
//!   without its parameter sets gets the last ones in front (`append_unit`).
//! - Admission: the textures are claimed by `GpuInput` (ring: `Readback`,
//!   encoder textures: `CodecInput`); the access unit is one copy into
//!   `AdmittedBytes` claimed as `CodecOutput` with the record budget of the
//!   frame contract (`mf_capture_output::capture_limit`), as the MF path.
//! - Rate (B-21): `RateParams` of BK4 (CBR, a buffer of one frame, full at
//!   the start), from the request's target and frame rate; a session retunes
//!   bit rate and frame rate without a restart.
//! - Failure: a picture that is no longer held skips the frame
//!   (`WouldBlock`, `needs_refresh`); any other GPU-path failure switches the
//!   capture to the CPU path for the session (`d3d_input::disable`) and ends
//!   this encoder with an error, so the host process puts it on its
//!   `refused` list and opens the next encoder (plan 4.10a).
//! - B-36: `recover_references` asks the session for a reference to the last
//!   confirmed picture instead of a key frame.
use std::io;

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::{AdmittedBytes, CapturedImage};
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::frame_size_caps::{
    avc_limits, check_video_dimensions, video_keyframe, video_parameter_sets,
};
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::rate_control::honour::EncoderRateState;
use crate::screen::core::rate_control::MIN_BITRATE_BPS;
use crate::screen::core::wire::invalid;
use crate::screen::os::mf_capture_output::capture_limit;
use crate::screen::os::mf_encoder::d3d_input::{self, GpuInput};

/// Rate control of a hardware encoder: constant bit rate, buffer of one
/// frame (bit rate / frame rate), buffer full at the start (B-21; R01
/// `nvenc_base.cpp:276-277` sets VBV = bitrate / fps). Identical to BK4's
/// `linux_os/hw_encode_common.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RateParams {
    pub bits_per_second: u32,
    pub buffer_bits: u32,
    pub initial_fullness: u32,
    pub fps: u32,
}

impl RateParams {
    /// `target_bps` within the floor and the `u32` of the drivers' rate
    /// fields; the ceiling is the encoder's (`VendorEncoder::ceiling`, r1),
    /// never a fixed value.
    pub(super) fn for_target(target_bps: u64, fps: u32) -> Self {
        let bits = target_bps.clamp(MIN_BITRATE_BPS, u64::from(u32::MAX));
        let fps = fps.max(1);
        let buffer = (bits / u64::from(fps)).max(1) as u32;
        Self {
            bits_per_second: bits as u32,
            buffer_bits: buffer,
            initial_fullness: buffer,
            fps,
        }
    }

    /// Whether `target` differs enough (2 %, the OpenH264 rule) to retune.
    pub(super) fn differs(&self, other: &Self) -> bool {
        let (a, b) = (
            i64::from(self.bits_per_second),
            i64::from(other.bits_per_second),
        );
        (a - b).abs() * 50 >= a || self.fps != other.fps
    }
}

/// The smallest H.264 level (`level_idc`) that carries `size` at `fps`
/// (Annex A table of `frame_size_avc.rs`), 62 when none does. Identical to
/// BK4's `avc_level_for`.
pub(super) fn avc_level_for(size: FrameSize, fps: u32) -> u8 {
    [30u32, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62]
        .into_iter()
        .find(|level| avc_limits(*level).is_some_and(|limits| limits.supports(size, fps.max(1))))
        .unwrap_or(62) as u8
}

// B-36 bookkeeping lives in `core::ref_recovery` (plan „Bildkette“ B-36).
#[cfg(test)]
pub(super) use crate::screen::core::ref_recovery::LtrPlan;
pub(super) use crate::screen::core::ref_recovery::{LtrSlots, Recovery, RefHistory};

/// One picture for a vendor session (named apart from the pipeline's
/// `capture_picture::Picture`, which the hull takes).
pub(super) struct VendorPicture<'a> {
    pub texture: &'a ID3D11Texture2D,
    pub frame_id: u64,
    pub idr: bool,
}

/// One opened vendor encoder session of a fixed size on the capture device.
pub(super) trait VendorSession {
    /// Encodes `picture`; `deliver` gets the coded access unit (valid
    /// during the call only). Returns whether it is an IDR.
    fn encode(
        &mut self,
        picture: VendorPicture<'_>,
        rate: &RateParams,
        deliver: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<bool>;
    /// B-36 (see `Recovery`).
    fn recover_references(&mut self, last_good: u64) -> Recovery;
    /// The bit-rate ceiling of the level the session writes and of the
    /// encoder's own report at `fps` (r1); `None`: neither is known (NVENC
    /// selects its level itself and reports no bit-rate limit).
    fn rate_ceiling(&self, _fps: u32) -> Option<u64> {
        None
    }
    /// The driver refused the last rate it was given (plan 2026-11-17).
    fn rate_refused(&self) -> bool {
        false
    }
}

/// The ScreenEncoder around a vendor session.
pub(super) struct VendorEncoder<S: VendorSession> {
    session: S,
    name: String,
    codec: CodecKind,
    size: FrameSize,
    grant: CaptureGrant,
    gpu: GpuInput,
    frames: u64,
    rate: Option<RateParams>,
    /// The ceiling of the codec pairing (`with_ceiling`, r1).
    ceiling_bps: u64,
    refresh: bool,
    /// HEVC VPS/SPS/PPS or the AV1 sequence header of the last key frame
    /// that carried them, put before a key frame without them.
    parameter_sets: Option<Vec<u8>>,
}

// SAFETY: as `AsyncEncoder`: one thread at a time uses the encoder; the
// D3D11 objects of `gpu` and the session's driver objects live on the
// capture device, which runs under multithread protection
// (`d3d_device::create_on`).
unsafe impl<S: VendorSession> Send for VendorEncoder<S> {}

impl<S: VendorSession> VendorEncoder<S> {
    pub(super) fn new(
        session: S,
        name: &str,
        codec: CodecKind,
        size: FrameSize,
        grant: CaptureGrant,
        gpu: GpuInput,
    ) -> Self {
        Self {
            session,
            name: name.into(),
            codec,
            size,
            grant,
            gpu,
            frames: 0,
            rate: None,
            ceiling_bps: u64::from(u32::MAX),
            refresh: false,
            parameter_sets: None,
        }
    }

    /// The highest bit rate of this encoder (r1, as Linux since merge W2):
    /// the ceiling of the codec pairing (`rate_control::codec_ceiling_bps`).
    pub(super) fn with_ceiling(mut self, bits_per_second: u64) -> Self {
        self.ceiling_bps = bits_per_second.clamp(MIN_BITRATE_BPS, u64::from(u32::MAX));
        self
    }

    /// The highest bit rate now: the pairing's and the session's (level it
    /// writes, the encoder's report), whichever is lower.
    pub(super) fn ceiling(&self, fps: u32) -> u64 {
        self.session
            .rate_ceiling(fps)
            .map_or(self.ceiling_bps, |session| session.min(self.ceiling_bps))
    }

    /// The rate for the request up to the ceiling; the running one while
    /// within 2 %.
    fn rate(&mut self, target_bps: u64, fps: u32) -> RateParams {
        let wanted = RateParams::for_target(target_bps.min(self.ceiling(fps)), fps);
        match self.rate {
            Some(current) if !current.differs(&wanted) => current,
            _ => {
                self.rate = Some(wanted);
                wanted
            }
        }
    }

    /// B-36: after a stream reset, a reference to `last_good` (a
    /// `frame_id` the viewer confirmed) instead of a key frame. Hand-over:
    /// the host process calls it before `encode_captured` when the request
    /// carries the last confirmed frame (see the BK12 report); until then
    /// nothing calls it.
    pub(super) fn recover_references(&mut self, last_good: u64) -> Recovery {
        if self.frames == 0 {
            return Recovery::KeyframeNeeded;
        }
        self.session.recover_references(last_good)
    }
}

/// Appends one access unit of `codec` to `bytes`: a HEVC or AV1 key frame
/// without its parameter sets gets the last ones in front (as BK6's
/// `hw_encode_common::video_prefix`); parameter sets in `unit` replace the
/// kept ones. Whether `unit` is a key frame by its stream marks.
pub(super) fn append_unit(
    bytes: &mut AdmittedBytes,
    maximum: usize,
    codec: CodecKind,
    kept: &mut Option<Vec<u8>>,
    unit: &[u8],
) -> io::Result<bool> {
    let keyframe = video_keyframe(codec, unit);
    let own = match codec {
        CodecKind::Hevc | CodecKind::Av1 => video_parameter_sets(codec, unit),
        CodecKind::H264 | CodecKind::Tiles => None,
    };
    let prefix = match (&own, &*kept, keyframe) {
        (None, Some(sets), true) if codec != CodecKind::H264 => sets.clone(),
        (None, None, true) if codec != CodecKind::H264 => {
            return Err(invalid(
                "vendor encoder gave its first key frame without parameters",
            ))
        }
        _ => Vec::new(),
    };
    if own.is_some() {
        *kept = own;
    }
    if bytes
        .len()
        .saturating_add(prefix.len())
        .saturating_add(unit.len())
        > maximum
    {
        return Err(invalid("vendor access unit exceeds its record budget"));
    }
    // AV1: the sequence header follows the temporal delimiter that opens
    // the temporal unit (AV1 7.5); HEVC: the parameter sets come first.
    let at = if codec == CodecKind::Av1 {
        temporal_delimiter(unit)
    } else {
        0
    };
    bytes.extend_from_slice(&unit[..at])?;
    bytes.extend_from_slice(&prefix)?;
    bytes.extend_from_slice(&unit[at..])?;
    Ok(keyframe)
}

/// The length of a leading AV1 temporal delimiter OBU (type 2, with its
/// size field 0 and no extension: 2 bytes; AV1 5.3.1, 5.6), else 0.
pub(super) fn temporal_delimiter(unit: &[u8]) -> usize {
    match unit {
        [header, 0, ..] if (header >> 3) & 0x0f == 2 && header & 0x06 == 0x02 => 2,
        _ => 0,
    }
}

/// The checked access unit of one picture.
pub(super) fn finish(
    bytes: AdmittedBytes,
    codec: CodecKind,
    size: FrameSize,
    keyframe: bool,
    wanted: bool,
) -> io::Result<CapturedEncodedFrame> {
    if bytes.is_empty() {
        return Err(invalid("vendor encoder gave no picture for the frame"));
    }
    if wanted && !keyframe {
        return Err(invalid("vendor encoder gave no key frame on request"));
    }
    check_video_dimensions(codec, bytes.as_slice(), size, FrameLimits::RESOURCE)?;
    Ok(CapturedEncodedFrame {
        records: CapturedRecords::one(bytes)?,
        keyframe,
    })
}

impl<S: VendorSession> ScreenEncoder for VendorEncoder<S> {
    fn recover_references(&mut self, last_good: u64) -> Recovery {
        VendorEncoder::recover_references(self, last_good)
    }

    fn encode_captured(
        &mut self,
        image: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        self.encode_picture(&Picture::Cpu(image.clone()), request)
    }

    /// A GPU picture goes in as its ring texture (scaled on the GPU where
    /// the sizes differ), a CPU picture converted on all cores and copied
    /// up (`GpuInput::texture`, as the Media Foundation texture path).
    fn encode_picture(
        &mut self,
        image: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let maximum = capture_limit(&self.grant, image, request, self.size)?;
        let texture = match self.gpu.texture(&self.grant, image) {
            Ok(texture) => texture,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.refresh = true;
                return Err(error);
            }
            Err(error) => {
                d3d_input::disable(&format!("{}: {error}", self.name));
                return Err(error);
            }
        };
        let rate = self.rate(request.target_bps, request.fps);
        let idr = request.keyframe || self.frames == 0;
        let mut bytes =
            AdmittedBytes::buffer(&self.grant, CaptureOwner::CodecOutput, maximum as u64)?;
        let picture = VendorPicture {
            texture: &texture,
            frame_id: request.frame_id,
            idr,
        };
        let (codec, kept) = (self.codec, &mut self.parameter_sets);
        let mut marked = false;
        let coded = self.session.encode(picture, &rate, &mut |unit| {
            marked |= append_unit(&mut bytes, maximum, codec, kept, unit)?;
            Ok(())
        });
        // The session has read the texture once its output is there.
        self.gpu.finish();
        let keyframe = coded? || marked;
        let frame = finish(bytes, self.codec, self.size, keyframe, idr)?;
        self.frames = self.frames.saturating_add(1);
        self.refresh = false;
        Ok(frame)
    }

    fn name(&self) -> String {
        format!("{} [GPU input (D3D11)]", self.name)
    }

    fn size(&self) -> FrameSize {
        self.size
    }

    fn needs_refresh(&self) -> bool {
        self.refresh
    }

    /// The rate the session runs at and whether it refused the last one.
    fn rate_state(&self) -> Option<EncoderRateState> {
        let rate = self.rate?;
        Some(EncoderRateState {
            applied_bps: u64::from(rate.bits_per_second),
            refused: self.session.rate_refused(),
        })
    }

    fn encode(
        &mut self,
        _frame: &BgraImage,
        _keyframe: bool,
        _target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "vendor encoder requires encode_captured",
        ))
    }
}
