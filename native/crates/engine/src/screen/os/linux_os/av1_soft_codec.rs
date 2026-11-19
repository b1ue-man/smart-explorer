//! Software AV1 of Linux hosts and viewers registered in the codec register
//! (plan 2026-11-14 „Bildkette“ BK6, B-25, B-38): an encoder factory around
//! BK4's `SvtAv1Encoder` and a decoder factory around its `Dav1dDecoder`
//! (`native_ffi/av1_soft.rs`), both present only where the system has the
//! shared object (`libSvtAv1Enc.so.1`, `libdav1d.so.7`/`.6`).
//!
//! Rank: the negotiation puts a software AV1 pairing after every H.264
//! pairing of its tier (B-30, `negotiate.rs`); whether a device carries a
//! size and rate in software is decided at run time by the encoding time
//! this session measures (B-02 of BK1, `rate_cadence.rs`), not by a test
//! measurement (4.10a). The stream: one temporal unit per picture in the
//! low-overhead OBU format (AV1 5.2), 8-bit 4:2:0 Main (`seq_profile` 0),
//! BT.709 limited range (the settings of `svt_settings`).
//!
//! SVT-AV1 1.7 changes no rate after `svt_av1_enc_init`, so a new rate
//! reopens the encoder (its first picture is a key frame with the sequence
//! header); to keep key frames rare the encoder reopens only when the
//! target leaves 70 %…140 % of the rate it runs with [P: own value].
use std::io;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::super::av1_soft::{
    dav1d_available, svt_library, Dav1dDecoder, SvtAv1Encoder, DAV1D_LIBRARIES,
};
use super::hw_encode_common::{HwH264Encoder, HwH264Session, HwInput, RateParams};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory};
use crate::screen::core::decoded_frame::DecodedFrame;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::frame_size_caps::check_video_dimensions;
use crate::screen::core::host_backend::{Chroma, ScreenDecoder, ScreenEncoder, YuvPicture};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::{
    i420_to_bgra, BgraImage, FrameSize, I420Image, YuvMatrix,
};
use crate::screen::core::wire::invalid;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

pub(super) const SVT_NAME: &str = "svt-av1";
pub(super) const DAV1D_NAME: &str = "dav1d";
/// The rate an SVT-AV1 before 4.0.0 runs with may drift this far before it
/// takes a new one (with a key frame, `SvtAv1Encoder::set_rate`).
const REOPEN_LOW: f64 = 0.7;
const REOPEN_HIGH: f64 = 1.4;
/// SVT-AV1 refuses pictures below 64 × 64 (assumption A-SVT-MIN).
const SVT_MIN_SIDE: u32 = 64;

type Factories = (Vec<Arc<dyn EncoderFactory>>, Vec<Arc<dyn DecoderFactory>>);

/// SVT-AV1 and dav1d where the system has a compatible version.
pub(super) fn codecs() -> Factories {
    static SVT: OnceLock<Option<&'static str>> = OnceLock::new();
    static DAV1D: OnceLock<bool> = OnceLock::new();
    let mut encoders: Vec<Arc<dyn EncoderFactory>> = Vec::new();
    let mut decoders: Vec<Arc<dyn DecoderFactory>> = Vec::new();
    if let Some(library) = *SVT.get_or_init(svt_library) {
        encoders.push(Arc::new(SvtAv1Factory { library }));
    }
    if *DAV1D.get_or_init(|| dav1d_available(&DAV1D_LIBRARIES)) {
        decoders.push(Arc::new(Dav1dFactory));
    }
    (encoders, decoders)
}

/// The SVT-AV1 encoder factory: AV1 Main in software.
pub(super) struct SvtAv1Factory {
    /// The SONAME `svt_library` found (`SVT_LIBRARIES`).
    library: &'static str,
}

impl SvtAv1Factory {
    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        if choice.codec != CodecKind::Av1 || choice.profile.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "SVT-AV1 encodes AV1 Main (4:2:0) only",
            ));
        }
        if size != size.even() || size.width < SVT_MIN_SIDE || size.height < SVT_MIN_SIDE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("SVT-AV1 cannot encode {}x{}", size.width, size.height),
            ));
        }
        let session = SvtSession {
            encoder: None,
            library: self.library,
            size,
            fps: fps.max(1),
            running: 0,
            rate_keyed: None,
        };
        let encoder = HwH264Encoder::new(session, SVT_NAME.into(), size, fps, grant);
        // Merge W2: the bit rate up to the pairing's level, no fixed ceiling.
        let ceiling = crate::screen::core::rate_control::codec_ceiling_bps(choice, size, fps);
        Ok(Box::new(
            encoder.for_codec(CodecKind::Av1).with_ceiling(ceiling),
        ))
    }
}

impl EncoderFactory for SvtAv1Factory {
    fn name(&self) -> String {
        SVT_NAME.into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::video(CodecKind::Av1, false, false).with_max(FrameSize::MAX)]
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

/// One SVT-AV1 encoder; a new rate goes to the running one (`set_rate`).
struct SvtSession {
    encoder: Option<SvtAv1Encoder>,
    library: &'static str,
    size: FrameSize,
    fps: u32,
    /// The bit rate the open encoder runs with.
    running: u32,
    /// Before SVT-AV1 4.0: when a lowered rate last came with its key frame.
    rate_keyed: Option<Instant>,
}

/// Review 2026-11-19 (F2): before SVT-AV1 4.0 a new rate needs a key frame
/// (`av1_soft_rate.rs`); lowered rates come with at most one such key frame
/// in this time, so a line that sinks step by step does not make a key frame
/// of every picture. A lowering in between waits for it; the stream's byte
/// budget keeps the ceiling meanwhile. Rises keep the band (`REOPEN_HIGH`).
const RATE_KEY_GAP: Duration = Duration::from_secs(1);
/// Review 2026-11-19 (J1): before 4.0 a lowered rate takes its key frame only
/// from this share of the running rate down (at least 10 % lower); a line
/// that wavers by less makes no key frame pulse. Above it the stream's byte
/// budget keeps the ceiling: it admits no picture beyond the user's rate.
const LOWER_STEP: (u64, u64) = (9, 10);

/// Whether `wanted` lies outside the band around `running` (B-25 note).
fn needs_reopen(running: u32, wanted: u32) -> bool {
    let (running, wanted) = (f64::from(running), f64::from(wanted));
    running <= 0.0 || wanted < running * REOPEN_LOW || wanted > running * REOPEN_HIGH
}

impl HwH264Session for SvtSession {
    fn wants_bgra(&self) -> bool {
        false
    }

    fn encode(
        &mut self,
        input: HwInput<'_>,
        idr: bool,
        rate: &RateParams,
        deliver: &mut dyn FnMut(&[&[u8]]) -> io::Result<()>,
    ) -> io::Result<()> {
        let HwInput::I420(yuv) = input else {
            return Err(invalid("SVT-AV1 takes I420"));
        };
        // Neither a key frame nor a new rate opens the encoder anew (review
        // 2026-11-19): `SvtAv1Encoder::encode_with` asks the key frame of the
        // running one and `set_rate` gives it the new rate
        // (`native_ffi/av1_soft_rate.rs`). SVT-AV1 4.x takes a rate on any
        // picture: every lowering and every rise past 2 % goes, as on the
        // other encoders (C10). Before 4.0.0 a rate needs a key picture of the
        // running encoder: a lowering by `LOWER_STEP` or more goes, at most one
        // a `RATE_KEY_GAP` (F2, J1), a rise only outside `needs_reopen`'s band.
        let wanted = rate.bits_per_second;
        if let Some(encoder) = self.encoder.as_mut() {
            let change = if encoder.rate_needs_keyframe() {
                let due = self
                    .rate_keyed
                    .is_none_or(|at| at.elapsed() >= RATE_KEY_GAP);
                let lower =
                    u64::from(wanted) * LOWER_STEP.1 <= u64::from(self.running) * LOWER_STEP.0;
                if lower && due {
                    self.rate_keyed = Some(Instant::now());
                }
                (lower && due) || (!lower && needs_reopen(self.running, wanted))
            } else {
                wanted < self.running
                    || u64::from(wanted.abs_diff(self.running)) * 50 >= u64::from(self.running)
            };
            if change {
                encoder.set_rate(u64::from(wanted));
                self.running = wanted;
            }
        }
        let reopened = self.encoder.is_none();
        if reopened {
            self.encoder = None;
            let opened = SvtAv1Encoder::open(
                self.library,
                self.size,
                self.fps,
                u64::from(rate.bits_per_second),
            )?;
            self.encoder = Some(opened);
            self.running = rate.bits_per_second;
        }
        let encoder = self
            .encoder
            .as_mut()
            .ok_or_else(|| invalid("SVT-AV1 encoder is missing"))?;
        let result = encoder.encode_with(yuv, idr || reopened, deliver);
        if result.is_err() {
            self.encoder = None;
        }
        result
    }
}

/// The dav1d decoder factory: AV1 Main (4:2:0) in software.
pub(super) struct Dav1dFactory;

impl DecoderFactory for Dav1dFactory {
    fn name(&self) -> String {
        DAV1D_NAME.into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::video(CodecKind::Av1, false, false).with_max(FrameSize::MAX)]
    }

    /// B-36 (r1): see `Dav1dScreenDecoder::keeps_references`.
    fn keeps_references(&self, codec: CodecKind) -> bool {
        codec == CodecKind::Av1
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if header.codec != CodecKind::Av1 || header.profile.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "dav1d of this build decodes AV1 Main (4:2:0) only",
            ));
        }
        let limits = header
            .frame_contract
            .map_or(FrameLimits::LEGACY, |contract| contract.limits);
        limits.check(header.size)?;
        Ok(Box::new(Dav1dScreenDecoder {
            decoder: Dav1dDecoder::open(&DAV1D_LIBRARIES)?,
            size: header.size,
            limits,
            picture: BgraImage::new_with_limits(header.size, limits)?,
            yuv: false,
            yuv_out: None,
            decoded: None,
        }))
    }
}

/// One dav1d decoder of a picture group. dav1d runs without frame delay
/// (`max_frame_delay` 1) and se hosts send no reordered pictures, so the
/// picture of a temporal unit is the one of its record (assumption
/// A-DAV1D-ORDER).
struct Dav1dScreenDecoder {
    decoder: Dav1dDecoder,
    size: FrameSize,
    limits: FrameLimits,
    picture: BgraImage,
    yuv: bool,
    yuv_out: Option<YuvPicture>,
    decoded: Option<DecodedFrame>,
}

impl ScreenDecoder for Dav1dScreenDecoder {
    fn name(&self) -> String {
        DAV1D_NAME.into()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        check_video_dimensions(CodecKind::Av1, &record.payload, self.size, self.limits)?;
        let Some(planes) = self
            .decoder
            .decode_expected(&record.payload, Some(self.size))?
        else {
            return Ok(false);
        };
        if planes.size != self.size {
            return Err(invalid("dav1d picture differs from its picture group"));
        }
        if self.yuv {
            let chroma_width = self.size.chroma().width as usize;
            self.yuv_out = Some(YuvPicture {
                y: planes.y,
                y_stride: self.size.width as usize,
                uv: Vec::new(),
                uv_stride: chroma_width,
                chroma: Chroma::I420 {
                    u: planes.u,
                    v: planes.v,
                },
                matrix: YuvMatrix::Bt709,
            });
        } else {
            i420_to_bgra(&planes, YuvMatrix::Bt709, &mut self.picture)?;
        }
        self.decoded = Some(DecodedFrame::from_record(record));
        Ok(true)
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
            self.yuv_out = None;
        }
    }

    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.yuv_out.clone().filter(|_| self.yuv)
    }

    /// r1: the planes dav1d's picture was copied into go to the viewer.
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.yuv_out.take().filter(|_| self.yuv)
    }

    /// The planes of a shown picture are the storage the next dav1d picture
    /// is copied into (`Dav1dDecoder::recycle`).
    fn recycle_yuv(&mut self, planes: YuvPicture) {
        if let Chroma::I420 { u, v } = planes.chroma {
            self.decoder.recycle(I420Image {
                size: self.size,
                y: planes.y,
                u,
                v,
            });
        }
    }

    /// B-36 (r1): dav1d keeps the eight reference slots of AV1 (7.20) and
    /// changes them only with `refresh_frame_flags` of frames it decodes;
    /// frames lost before a continuation never reach it, so the slot the
    /// host names for the confirmed picture still holds that picture
    /// (assumption W3-A5: the host names only slots the viewer holds). An
    /// empty slot is a decoding error, never a wrong picture.
    fn keeps_references(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B-25 note: the SVT-AV1 encoder reopens only outside 70 %…140 %.
    #[test]
    fn bildkette_task_codec_svt_reopens_only_on_a_large_rate_change() {
        assert!(needs_reopen(0, 6_000_000));
        assert!(!needs_reopen(6_000_000, 6_000_000));
        assert!(!needs_reopen(6_000_000, 4_300_000));
        assert!(!needs_reopen(6_000_000, 8_300_000));
        assert!(needs_reopen(6_000_000, 4_000_000));
        assert!(needs_reopen(6_000_000, 9_000_000));
        let offers = SvtAv1Factory {
            library: "libSvtAv1Enc.so.3",
        }
        .offers();
        assert_eq!(offers.len(), 1);
        assert_eq!(
            (offers[0].codec, offers[0].profile, offers[0].hardware),
            (CodecKind::Av1, None, false)
        );
        assert_eq!(Dav1dFactory.offers(), offers);
    }
}
