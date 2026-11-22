//! What the Linux hardware H.264 encoders share (BK4, B-21 analogously):
//! the rate parameters (CBR, a buffer of one frame), the level, the input
//! form, and the `ScreenEncoder` around one hardware session with the
//! capture admission of the host process: every product copy (I420
//! conversion, access unit) is claimed before it is allocated, as in
//! `openh264_capture.rs`. An IDR whose access unit lacks SPS and PPS gets
//! the ones of the first IDR in front, so every picture group stays
//! decodable on its own (a new group opens a new decoder at the viewer).
//! BK6: the same wrapper carries HEVC and AV1 sessions (`for_codec`), with
//! the key-frame mark, the parameter sets and the parameter check of
//! `frame_size_hevc_av1.rs` instead of the H.264 ones. Merge W2: the bit
//! rate is bounded by the pairing's level (`with_ceiling`), not by a fixed
//! value; GPU pictures of the DMA-BUF capture and the reference recovery of
//! B-36 pass through (`hw_encode_dmabuf.rs`, `HwH264Session`).
use std::io;

use super::h264_bits::{contains_nal, parameter_sets, NAL_IDR, NAL_PPS, NAL_SPS};
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
use crate::screen::core::pixel_convert::{
    bgra_to_i420, BgraImage, BgraView, FrameSize, I420Image, YuvMatrix,
};
use crate::screen::core::rate_control::MIN_BITRATE_BPS;
use crate::screen::core::ref_recovery::Recovery;
use crate::screen::core::wire::invalid;
use crate::screen::core::wire_binary::MAX_RECORD_BYTES;

/// Rate control of a hardware encoder: constant bit rate, buffer of one frame
/// full at the start (B-21; R01 `nvenc_base.cpp:260-283`: VBV = rate / fps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RateParams {
    pub bits_per_second: u32,
    pub buffer_bits: u32,
    pub initial_fullness: u32,
    pub fps: u32,
}

impl RateParams {
    /// `target_bps` within the floor and the `u32` of the drivers' rate
    /// fields; the codec's ceiling is the encoder's (`HwH264Encoder::rate`).
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

    /// Whether `other` lowers this rate at all (review 2026-11-19 C10).
    pub(super) fn lowered_by(&self, other: &Self) -> bool {
        other.bits_per_second < self.bits_per_second
    }
}

/// MaxBR in bit/s of the H.264 level `avc_level_for` names for `size` at
/// `fps` (Table A-1, cpbBrVclFactor of `profile`; merge W2).
pub(super) fn avc_level_ceiling(
    size: FrameSize,
    fps: u32,
    profile: crate::screen::core::negotiate::H264Profile,
) -> Option<u64> {
    crate::screen::core::rate_control::avc_level_max_bitrate_bps(
        u32::from(avc_level_for(size, fps)),
        profile,
    )
}

/// The smallest H.264 level (`level_idc`) that carries `size` at `fps`
/// (Annex A table of `frame_size_avc.rs`), 62 when none does.
pub(super) fn avc_level_for(size: FrameSize, fps: u32) -> u8 {
    [30u32, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62]
        .into_iter()
        .find(|level| avc_limits(*level).is_some_and(|limits| limits.supports(size, fps.max(1))))
        .unwrap_or(62) as u8
}

/// The picture a session receives: BGRA for a session that converts on
/// the GPU (VA video processor, B-18), else the CPU's I420 (BT.709), or
/// the picture `HwH264Session::import_dmabuf` already put into the
/// session's input (BK8b).
pub(super) enum HwInput<'a> {
    Bgra(BgraView<'a>),
    I420(&'a I420Image),
    Prepared,
}

/// A DMA-BUF picture of the capture, held until dropped (BK8b): while it
/// lives the PipeWire buffer stays with this program, dropping it gives the
/// buffer back to the compositor. Implemented by the PipeWire stream.
pub(in crate::screen::os) trait HeldDmaBuf {
    fn frame(&self) -> &super::vaapi_vpp::DmaBufFrame;
    /// Source and generation the picture was captured under.
    fn identity(
        &self,
    ) -> (
        &crate::screen::core::capture_admission::SourceKey,
        crate::screen::core::capture_admission::CaptureGeneration,
    );
}

/// One opened hardware H.264 encoder of a fixed size.
pub(super) trait HwH264Session: Send {
    /// Whether `encode` takes BGRA (else I420).
    fn wants_bgra(&self) -> bool;
    /// Whether `import_dmabuf` works (a VA session with video processor).
    fn imports_dmabuf(&self) -> bool {
        false
    }
    /// Imports `frame`, converts and scales it into the session's input and
    /// waits until the GPU read it; afterwards the buffer may go back.
    fn import_dmabuf(&mut self, _frame: &super::vaapi_vpp::DmaBufFrame) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "hardware encoder takes no DMA-BUF",
        ))
    }
    /// The frame id of the next picture (B-36: NVENC stamps it).
    fn set_frame_id(&mut self, _frame_id: u64) {}
    /// B-36: the next picture may reference only `last_good` or older; a
    /// session without reference control needs a key frame.
    fn recover_references(&mut self, _last_good: u64) -> Recovery {
        Recovery::KeyframeNeeded
    }
    /// See `ScreenEncoder::confirm_reference` (NVENC, Gemini 8/3.1).
    fn confirm_reference(&mut self, _frame_id: u64) {}
    /// The MaxBR of the level the session writes for its size at `fps`
    /// (merge W2, as the Media Foundation encoder's `opened_limit`): a VA
    /// session names its level itself; NVENC chooses it (`AUTOSELECT`).
    fn level_ceiling(&self, _fps: u32) -> Option<u64> {
        None
    }
    /// Encodes one picture, an IDR when `idr`; `deliver` gets the coded
    /// access unit as the session's own memory segments, valid during the
    /// call only.
    fn encode(
        &mut self,
        input: HwInput<'_>,
        idr: bool,
        rate: &RateParams,
        deliver: &mut dyn FnMut(&[&[u8]]) -> io::Result<()>,
    ) -> io::Result<()>;
}

/// The bytes of one access unit before they become a record.
trait AuSink {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn bytes(&self) -> &[u8];
}

impl AuSink for AdmittedBytes {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes)
    }
    fn bytes(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AuSink for Vec<u8> {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }
    fn bytes(&self) -> &[u8] {
        self
    }
}

/// What has to go in front of `segments`: nothing, or for an IDR without
/// its own SPS and PPS the cached ones (an error without a cache).
pub(super) fn prefix_for<'a>(
    segments: &[&[u8]],
    idr: bool,
    cached: Option<&'a [u8]>,
) -> io::Result<&'a [u8]> {
    let has = |kind| segments.iter().any(|segment| contains_nal(segment, kind));
    if !idr || (has(NAL_SPS) && has(NAL_PPS)) {
        return Ok(&[]);
    }
    cached.ok_or_else(|| invalid("hardware encoder gave its first IDR without SPS and PPS"))
}

/// What goes in front of an HEVC or AV1 key frame `bytes` without its own
/// parameter sets (VPS/SPS/PPS, the sequence header OBU): the cached ones
/// (BK6), so every picture group stays decodable on its own.
fn video_prefix<'a>(
    codec: CodecKind,
    bytes: &[u8],
    keyframe: bool,
    cached: Option<&'a [u8]>,
) -> io::Result<&'a [u8]> {
    if !keyframe || video_parameter_sets(codec, bytes).is_some() {
        return Ok(&[]);
    }
    cached.ok_or_else(|| invalid("hardware encoder gave its first key frame without parameters"))
}

/// The ScreenEncoder around a hardware session; H.264 unless `for_codec`
/// names HEVC or AV1 (BK6: NVENC, SVT-AV1 share it).
pub(super) struct HwH264Encoder<S: HwH264Session> {
    session: S,
    name: String,
    size: FrameSize,
    fps: u32,
    grant: Option<CaptureGrant>,
    frames: u64,
    has_input: bool,
    rate: Option<RateParams>,
    parameter_sets: Option<Vec<u8>>,
    codec: CodecKind,
    /// The highest bit rate (merge W2: the pairing's level, `with_ceiling`).
    ceiling_bps: u64,
}

impl<S: HwH264Session> HwH264Encoder<S> {
    pub(super) fn new(
        session: S,
        name: String,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> Self {
        Self {
            session,
            name,
            size,
            fps: fps.max(1),
            grant,
            frames: 0,
            has_input: false,
            rate: None,
            parameter_sets: None,
            codec: CodecKind::H264,
            ceiling_bps: u64::from(u32::MAX),
        }
    }

    /// The highest bit rate of this encoder (merge W2): the ceiling of the
    /// codec pairing (`rate_control::codec_ceiling_bps`, the MaxBR of the
    /// level both sides carry), never a fixed value.
    pub(super) fn with_ceiling(mut self, bits_per_second: u64) -> Self {
        self.ceiling_bps = bits_per_second.clamp(MIN_BITRATE_BPS, u64::from(u32::MAX));
        self
    }

    /// The highest bit rate now: the pairing's and the one of the level the
    /// session writes, whichever is lower.
    pub(super) fn ceiling(&self) -> u64 {
        self.session
            .level_ceiling(self.fps)
            .map_or(self.ceiling_bps, |level| level.min(self.ceiling_bps))
    }

    /// The same encoder for an HEVC or AV1 session.
    pub(super) fn for_codec(self, codec: CodecKind) -> Self {
        Self { codec, ..self }
    }

    /// The rate for `target_bps` up to the ceiling; the running one while
    /// within 2 %.
    fn rate(&mut self, target_bps: u64) -> RateParams {
        let wanted = RateParams::for_target(target_bps.min(self.ceiling()), self.fps);
        match self.rate {
            // Review 2026-11-19 C10: the 2 % band holds back rises only.
            Some(current) if !current.differs(&wanted) && !current.lowered_by(&wanted) => current,
            _ => {
                self.rate = Some(wanted);
                wanted
            }
        }
    }

    /// Runs one picture through the session into a sink from `make`
    /// (given the exact length); returns the sink and the key frame flag.
    fn run<K: AuSink>(
        &mut self,
        input: HwInput<'_>,
        (wanted, frame_id): (bool, u64),
        target_bps: u64,
        maximum: usize,
        make: &mut dyn FnMut(usize) -> io::Result<K>,
    ) -> io::Result<(K, bool)> {
        let idr = wanted || self.frames == 0;
        let rate = self.rate(target_bps);
        self.session.set_frame_id(frame_id);
        let cached = self.parameter_sets.clone();
        let codec = self.codec;
        let mut out: Option<K> = None;
        self.session.encode(input, idr, &rate, &mut |segments| {
            let prefix = if codec == CodecKind::H264 {
                prefix_for(segments, idr, cached.as_deref())?
            } else {
                // NVENC and SVT-AV1 deliver one segment: no copy then.
                let joined: std::borrow::Cow<'_, [u8]> = match segments {
                    [one] => std::borrow::Cow::Borrowed(*one),
                    many => std::borrow::Cow::Owned(many.concat()),
                };
                let key = video_keyframe(codec, &joined);
                video_prefix(codec, &joined, key, cached.as_deref())?
            };
            let total = segments
                .iter()
                .try_fold(prefix.len(), |sum, segment| sum.checked_add(segment.len()))
                .filter(|total| *total <= maximum)
                .ok_or_else(|| invalid("hardware access unit exceeds its record budget"))?;
            let mut sink = make(total)?;
            sink.append(prefix)?;
            for segment in segments {
                sink.append(segment)?;
            }
            out = Some(sink);
            Ok(())
        })?;
        let sink = out.ok_or_else(|| invalid("hardware encoder gave no picture for the frame"))?;
        let bytes = sink.bytes();
        if bytes.is_empty() {
            return Err(invalid("hardware encoder gave no picture for the frame"));
        }
        let h264 = codec == CodecKind::H264;
        let keyframe = if h264 {
            contains_nal(bytes, NAL_IDR)
        } else {
            video_keyframe(codec, bytes)
        };
        if idr && !keyframe {
            return Err(invalid("hardware encoder gave no IDR on request"));
        }
        if keyframe {
            let sets = if h264 {
                parameter_sets(bytes)
            } else {
                video_parameter_sets(codec, bytes)
            };
            if let Some(sets) = sets {
                self.parameter_sets = Some(sets);
            }
        }
        check_video_dimensions(codec, bytes, self.size, FrameLimits::RESOURCE)?;
        self.frames = self
            .frames
            .checked_add(1)
            .ok_or_else(|| invalid("hardware encoder frame counter overflow"))?;
        Ok((sink, keyframe))
    }
}

#[path = "hw_encode_dmabuf.rs"]
mod dmabuf;

/// The record budget of an admitted frame (as `openh264_capture.rs`).
fn capture_limit(
    grant: &CaptureGrant,
    frame: &CapturedImage,
    request: &StageRequest,
    size: FrameSize,
) -> io::Result<usize> {
    grant.check_identity(frame.source(), frame.generation())?;
    if frame.size() != size || request.size != size {
        return Err(invalid(
            "capture geometry differs from the hardware encoder",
        ));
    }
    if grant.plan().contract_binding() != Some(request.frame_contract) {
        return Err(invalid(
            "hardware encoder request differs from its admitted contract",
        ));
    }
    let packet =
        crate::screen::core::frame_size_caps::PacketBudget::for_contract(request.frame_contract)?;
    Ok(packet.record_bytes.min(MAX_RECORD_BYTES))
}

impl<S: HwH264Session> ScreenEncoder for HwH264Encoder<S> {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| invalid("hardware encoder has no capture grant"))?;
        let maximum = capture_limit(&grant, frame, request, self.size)?;
        let converted = if self.session.wants_bgra() {
            None
        } else {
            Some(super::super::openh264_capture::admitted_i420(
                &grant, frame,
            )?)
        };
        let input = match &converted {
            Some(yuv) => HwInput::I420(yuv.resource()),
            None => HwInput::Bgra(frame.view()),
        };
        let mut make =
            |length: usize| AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, length as u64);
        let (bytes, keyframe) = self.run(
            input,
            (request.keyframe, request.frame_id),
            request.target_bps,
            maximum,
            &mut make,
        )?;
        self.has_input = true;
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::one(bytes)?,
            keyframe,
        })
    }

    /// BK8b/merge W2: a DMA-BUF picture of the capture goes to the GPU
    /// import (`hw_encode_dmabuf.rs`); every other picture as before.
    fn encode_picture(
        &mut self,
        picture: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        self.encode_gpu_picture(picture, request)
    }

    fn recover_references(&mut self, last_good: u64) -> Recovery {
        self.session.recover_references(last_good)
    }

    fn confirm_reference(&mut self, frame_id: u64) {
        self.session.confirm_reference(frame_id);
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn size(&self) -> FrameSize {
        self.size
    }

    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        if self.grant.is_some() {
            return Err(invalid(
                "admitted hardware encoder requires encode_captured",
            ));
        }
        if frame.size() != self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the hardware encoder",
            ));
        }
        let converted = if self.session.wants_bgra() {
            None
        } else {
            Some(bgra_to_i420(&frame.view(), YuvMatrix::Bt709)?)
        };
        let input = match &converted {
            Some(yuv) => HwInput::I420(yuv),
            None => HwInput::Bgra(frame.view()),
        };
        let mut make = |length: usize| -> io::Result<Vec<u8>> { Ok(Vec::with_capacity(length)) };
        let frame_id = self.frames;
        let (bytes, keyframe) = self.run(
            input,
            (keyframe, frame_id),
            target_bps,
            MAX_RECORD_BYTES,
            &mut make,
        )?;
        Ok(EncodedFrame {
            records: vec![bytes],
            keyframe,
        })
    }
}
