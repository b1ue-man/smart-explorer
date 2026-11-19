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
/// The rate an encoder runs with may drift this far before it reopens.
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

/// One SVT-AV1 encoder, reopened when the rate drifts too far.
struct SvtSession {
    encoder: Option<SvtAv1Encoder>,
    library: &'static str,
    size: FrameSize,
    fps: u32,
    /// The bit rate the open encoder runs with.
    running: u32,
}

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
        // A key frame no longer opens the encoder anew (`SvtAv1Encoder::
        // encode_with` asks it of the running one, Codex round 4, finding 10).
        let reopened = self.encoder.is_none() || needs_reopen(self.running, rate.bits_per_second);
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
