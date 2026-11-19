//! OpenH264 encoder; the productive entrypoint keeps every product copy admitted.
//!
//! Plan 2026-11-14 bildkette (BK3b): the bit rate is bounded by the ceiling
//! of the codec pairing the encoder was opened for (`rate_cadence::
//! codec_ceiling_bps`, the H.264 level both sides report, B-06), not by a
//! fixed 240 Mbit/s; the BGRA → I420 conversion runs on all cores
//! (`openh264_capture::admitted_i420`).
use super::codec_inventory::openh264_encoder_offers;
use super::openh264_capture::{admitted_i420, capture_limit, copy_access_unit};
use super::openh264_codec::{coder_error, load, OPENH264_NAME};
use super::openh264_install::OpenH264Store;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::codec_registry::EncoderFactory;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::{bgra_to_i420_parallel, BgraImage, FrameSize, YuvMatrix};
use crate::screen::core::rate_control::{codec_ceiling_bps, MIN_BITRATE_BPS};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, Profile, RateControlMode,
    UsageType, VuiConfig,
};
use openh264::formats::YUVSlices;
use openh264::OpenH264API;
use std::io;
use std::os::raw::{c_int, c_void};

// ISVCEncoder SetOption values pinned by openh264-sys2 0.9.9.
const ENCODER_OPTION_BITRATE: c_int = 5;
const ENCODER_OPTION_MAX_BITRATE: c_int = 6;
const SPATIAL_LAYER_0: c_int = 0;
const SPATIAL_LAYER_ALL: c_int = 4;
#[repr(C)]
struct BitrateInfo {
    layer: c_int,
    bitrate: c_int,
}

pub struct OpenH264EncoderFactory {
    pub(super) store: OpenH264Store,
}

impl EncoderFactory for OpenH264EncoderFactory {
    fn name(&self) -> String {
        OPENH264_NAME.into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        openh264_encoder_offers()
    }

    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(Box::new(self.create(choice, size, fps, None)?))
    }

    fn open_admitted(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(Box::new(self.create(choice, size, fps, Some(grant))?))
    }
}

impl OpenH264EncoderFactory {
    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<OpenH264Encoder> {
        let compatible = choice.codec == CodecKind::H264
            && choice
                .profile
                .is_some_and(|wanted| wanted.decodes(H264Profile::ConstrainedBaseline));
        if !compatible {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "OpenH264 encodes Constrained Baseline only",
            ));
        }
        let fits = openh264_encoder_offers().iter().any(|offer| {
            size.width <= offer.max_size().width && size.height <= offer.max_size().height
        });
        if !fits || size != size.even() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("OpenH264 cannot encode {}x{}", size.width, size.height),
            ));
        }
        let baseline = CodecChoice {
            profile: Some(H264Profile::ConstrainedBaseline),
            ..*choice
        };
        Ok(OpenH264Encoder {
            api: Some(load(&self.store)?),
            encoder: None,
            size,
            fps: fps.max(1),
            bitrate: 0,
            limit: codec_ceiling_bps(&baseline, size, fps.max(1)),
            grant,
        })
    }
}

/// One OpenH264 encoder; it starts with the bit rate of the first frame.
struct OpenH264Encoder {
    api: Option<OpenH264API>,
    encoder: Option<Encoder>,
    size: FrameSize,
    fps: u32,
    /// The bit rate the encoder runs with.
    bitrate: c_int,
    /// Ceiling of the codec pairing the encoder was opened for.
    limit: u64,
    grant: Option<CaptureGrant>,
}

/// `target_bps` within the floor of the rate control, the ceiling `limit`
/// and the `int` of the OpenH264 API.
fn clamp_bitrate(target_bps: u64, limit: u64) -> c_int {
    let ceiling = limit.min(c_int::MAX as u64).max(MIN_BITRATE_BPS);
    target_bps.clamp(MIN_BITRATE_BPS, ceiling) as c_int
}

impl OpenH264Encoder {
    fn started(&mut self, bitrate: c_int) -> io::Result<&mut Encoder> {
        if self.encoder.is_none() {
            let api = self
                .api
                .take()
                .ok_or_else(|| io::Error::other("OpenH264 encoder lost its library"))?;
            let config = EncoderConfig::new()
                .usage_type(UsageType::ScreenContentRealTime)
                .rate_control_mode(RateControlMode::Bitrate)
                .bitrate(BitRate::from_bps(bitrate as u32))
                .max_frame_rate(FrameRate::from_hz(self.fps as f32))
                .skip_frames(false)
                .profile(Profile::Baseline)
                .complexity(Complexity::Low)
                .vui(VuiConfig::bt709());
            let encoder = Encoder::with_api_config(api, config).map_err(coder_error)?;
            self.encoder = Some(encoder);
            self.bitrate = bitrate;
        }
        self.encoder
            .as_mut()
            .ok_or_else(|| io::Error::other("OpenH264 encoder is missing"))
    }

    /// Sets a changed bit rate (at least 2 % off) on the running encoder:
    /// the maximum first when it rises, the target first when it falls, so
    /// the target never exceeds the maximum. A refused option keeps the old
    /// rate. Review 2026-11-19 C10: a lowered rate is set however small the
    /// step (the 2 % band holds back rises only), and a refused lowering
    /// fails the picture, so the stage opens a fresh encoder at that rate or
    /// the next one instead of running above it.
    fn retune(&mut self, target: c_int) -> io::Result<()> {
        let current = self.bitrate;
        let lowered = target < current;
        if !lowered && (i64::from(target) - i64::from(current)).abs() * 50 < i64::from(current) {
            return Ok(());
        }
        let Some(encoder) = self.encoder.as_mut() else {
            return Ok(());
        };
        let max = [
            (ENCODER_OPTION_MAX_BITRATE, SPATIAL_LAYER_ALL),
            (ENCODER_OPTION_MAX_BITRATE, SPATIAL_LAYER_0),
        ];
        let rate = [
            (ENCODER_OPTION_BITRATE, SPATIAL_LAYER_ALL),
            (ENCODER_OPTION_BITRATE, SPATIAL_LAYER_0),
        ];
        let order = if target > current {
            [max, rate]
        } else {
            [rate, max]
        };
        let mut accepted = true;
        for (option, layer) in order.into_iter().flatten() {
            let mut info = BitrateInfo {
                layer,
                bitrate: target,
            };
            // SAFETY: the encoder encoded a frame, so it is initialized;
            // `info` has the layout of `SBitrateInfo` and outlives the call,
            // which copies the values.
            let result = unsafe {
                encoder
                    .raw_api()
                    .set_option(option, (&mut info as *mut BitrateInfo).cast::<c_void>())
            };
            accepted &= result == 0;
        }
        if accepted {
            self.bitrate = target;
        } else if lowered && (i64::from(current) - i64::from(target)) * 50 >= i64::from(current) {
            return Err(io::Error::other(format!(
                "OpenH264 refused to lower its bit rate from {current} to {target} bit/s"
            )));
        }
        Ok(())
    }
}

impl ScreenEncoder for OpenH264Encoder {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| io::Error::other("OpenH264 encoder has no capture grant"))?;
        let maximum = capture_limit(&grant, frame, request, self.size)?;
        let target = clamp_bitrate(request.target_bps, self.limit);
        let first = self.encoder.is_none();
        if !first {
            self.retune(target)?;
        }
        let yuv = admitted_i420(&grant, frame)?;
        let yuv = yuv.resource();
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let encoder = self.started(target)?;
        if request.keyframe && !first {
            encoder.force_intra_frame();
        }
        let source = YUVSlices::new(
            (&yuv.y, &yuv.u, &yuv.v),
            (width, height),
            (width, width / 2, width / 2),
        );
        let stream = encoder.encode(&source).map_err(coder_error)?;
        let keyframe = stream.frame_type() == FrameType::IDR;
        if request.keyframe && !keyframe {
            return Err(io::Error::other(
                "OpenH264 gave no IDR for a requested key frame",
            ));
        }
        let records = if stream.frame_type() == FrameType::Skip {
            crate::screen::core::capture_records::CapturedRecords::empty(&grant)?
        } else {
            copy_access_unit(&grant, &stream, maximum)?
        };
        Ok(CapturedEncodedFrame {
            keyframe: keyframe && !records.is_empty(),
            records,
        })
    }

    fn name(&self) -> String {
        OPENH264_NAME.into()
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
            return Err(io::Error::other(
                "admitted OpenH264 encoder requires encode_captured",
            ));
        }
        if frame.size() != self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the OpenH264 encoder",
            ));
        }
        let target = clamp_bitrate(target_bps, self.limit);
        let first = self.encoder.is_none();
        if !first {
            self.retune(target)?;
        }
        let yuv = bgra_to_i420_parallel(&frame.view(), YuvMatrix::Bt709, FrameLimits::LEGACY)?;
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let encoder = self.started(target)?;
        if keyframe && !first {
            encoder.force_intra_frame();
        }
        let source = YUVSlices::new(
            (&yuv.y, &yuv.u, &yuv.v),
            (width, height),
            (width, width / 2, width / 2),
        );
        let stream = encoder.encode(&source).map_err(coder_error)?;
        let idr = stream.frame_type() == FrameType::IDR;
        let skipped = stream.frame_type() == FrameType::Skip;
        let bytes = stream.to_vec();
        if keyframe && !idr {
            return Err(io::Error::other(
                "OpenH264 gave no IDR for a requested key frame",
            ));
        }
        if skipped || bytes.is_empty() {
            return Ok(EncodedFrame {
                records: Vec::new(),
                keyframe: false,
            });
        }
        Ok(EncodedFrame {
            records: vec![bytes],
            keyframe: idr,
        })
    }
}
