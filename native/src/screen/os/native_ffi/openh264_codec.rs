//! OpenH264 encoder and decoder (SCR-M3, decision C2) through the crate
//! `openh264` with the feature `libloading`: the Cisco library is loaded with
//! `OpenH264API::from_blob_path` (the crate checks it against its own list)
//! only after it matched the pin of this build right before
//! (`OpenH264Store::verified_library`). Each encoder and decoder holds its
//! own load; dropping the last one unloads the library, so switching
//! OpenH264 off ends its use as soon as the running coders are dropped. The
//! encoder takes BGRA, converts it to I420 (BT.709, `pixel_convert`) and
//! gives one Annex B access unit per frame: Constrained Baseline without B
//! frames, an IDR on request, the bit rate set at run time through the raw
//! encoder option (`ENCODER_OPTION_BITRATE`). The decoder turns I420 back to
//! BGRA, or after `prefer_yuv(true)` (SCR-M7, the viewer's GPU path) keeps
//! the I420 planes and skips that conversion. OpenH264 encodes at most 3840 × 2160 (`openh264` 0.9.9,
//! `encoder.rs`).
use std::io;
use std::os::raw::{c_int, c_void};
use std::sync::Arc;

use openh264::decoder::{Decoder, DecoderConfig};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, Profile, RateControlMode,
    UsageType, VuiConfig,
};
use openh264::formats::{YUVSlices, YUVSource};
use openh264::OpenH264API;

use super::codec_inventory::{openh264_decoder_offers, openh264_encoder_offers};
use super::openh264_install::OpenH264Store;
use crate::screen::core::codec_registry::{
    DecoderFactory, EncoderFactory, OpenH264Report, OpenH264State,
};
use crate::screen::core::host_backend::{
    Chroma, EncodedFrame, ScreenDecoder, ScreenEncoder, YuvPicture,
};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::{
    bgra_to_i420, i420_to_bgra, BgraImage, FrameSize, I420Image, YuvMatrix,
};
use crate::screen::core::rate_control::{MAX_BITRATE_BPS, MIN_BITRATE_BPS};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

/// Name in the statistics and in `se screen codec`.
pub(crate) const OPENH264_NAME: &str = "openh264";
// `ISVCEncoder::SetOption` ids and layers of OpenH264 (`openh264-sys2`
// 0.9.9, `src/generated/types.rs` lines 263, 264, 367 and 371).
const ENCODER_OPTION_BITRATE: c_int = 5;
const ENCODER_OPTION_MAX_BITRATE: c_int = 6;
const SPATIAL_LAYER_0: c_int = 0;
const SPATIAL_LAYER_ALL: c_int = 4;

/// `SBitrateInfo` of OpenH264 (`TagBitrateInfo`, `types.rs` line 928).
#[repr(C)]
struct BitrateInfo {
    layer: c_int,
    bitrate: c_int,
}

fn coder_error(error: openh264::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// Loads the library of `store` after its pin check.
fn load(store: &OpenH264Store) -> io::Result<OpenH264API> {
    let path = store.verified_library()?;
    OpenH264API::from_blob_path(&path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

/// The OpenH264 coders of `store` when the user switched them on, and the
/// state for the report; `verify` checks the file against its pin first.
pub(crate) fn codecs(
    store: OpenH264Store,
    verify: bool,
) -> (
    Vec<Arc<dyn EncoderFactory>>,
    Vec<Arc<dyn DecoderFactory>>,
    OpenH264Report,
) {
    let report = if verify {
        store.report()
    } else if store.switched_on() {
        OpenH264Report::new(OpenH264State::Enabled, None)
    } else if store.library_path().is_some_and(|path| path.is_file()) {
        OpenH264Report::new(OpenH264State::Disabled, None)
    } else {
        OpenH264Report::new(OpenH264State::Absent, None)
    };
    if report.state != OpenH264State::Enabled {
        return (Vec::new(), Vec::new(), report);
    }
    let encoder: Arc<dyn EncoderFactory> = Arc::new(OpenH264EncoderFactory {
        store: store.clone(),
    });
    let decoder: Arc<dyn DecoderFactory> = Arc::new(OpenH264DecoderFactory { store });
    (vec![encoder], vec![decoder], report)
}

pub(crate) struct OpenH264EncoderFactory {
    store: OpenH264Store,
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
        if choice.profile != Some(H264Profile::ConstrainedBaseline) {
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
        Ok(Box::new(OpenH264Encoder {
            api: Some(load(&self.store)?),
            encoder: None,
            size,
            fps: fps.max(1),
            bitrate: 0,
        }))
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
}

fn clamp_bitrate(target_bps: u64) -> c_int {
    target_bps.clamp(MIN_BITRATE_BPS, MAX_BITRATE_BPS) as c_int
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
    /// rate.
    fn retune(&mut self, target: c_int) {
        let current = self.bitrate;
        if (i64::from(target) - i64::from(current)).abs() * 50 < i64::from(current) {
            return;
        }
        let Some(encoder) = self.encoder.as_mut() else {
            return;
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
        }
    }
}

impl ScreenEncoder for OpenH264Encoder {
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
        if frame.size() != self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the OpenH264 encoder",
            ));
        }
        let target = clamp_bitrate(target_bps);
        let first = self.encoder.is_none();
        if !first {
            self.retune(target);
        }
        let yuv = bgra_to_i420(&frame.view(), YuvMatrix::Bt709)?;
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

pub(crate) struct OpenH264DecoderFactory {
    store: OpenH264Store,
}

impl DecoderFactory for OpenH264DecoderFactory {
    fn name(&self) -> String {
        OPENH264_NAME.into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        openh264_decoder_offers()
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        let decoder = Decoder::with_api_config(load(&self.store)?, DecoderConfig::new())
            .map_err(coder_error)?;
        Ok(Box::new(OpenH264Decoder {
            decoder,
            size: header.size,
            planes: I420Image {
                size: header.size,
                y: Vec::new(),
                u: Vec::new(),
                v: Vec::new(),
            },
            picture: BgraImage::new(header.size)?,
            yuv: false,
            yuv_out: None,
        }))
    }
}

/// One OpenH264 decoder of a picture group of `size`.
struct OpenH264Decoder {
    decoder: Decoder,
    size: FrameSize,
    planes: I420Image,
    picture: BgraImage,
    /// `prefer_yuv(true)`: hand the planes out instead of converting.
    yuv: bool,
    yuv_out: Option<YuvPicture>,
}

/// Copies `rows` rows of `width` bytes at `stride` from `source`.
fn copy_plane(
    source: &[u8],
    stride: usize,
    width: usize,
    rows: usize,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    let needed = stride
        .checked_mul(rows.saturating_sub(1))
        .and_then(|bytes| bytes.checked_add(width));
    if stride < width || needed.is_none_or(|needed| source.len() < needed) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decoded OpenH264 plane is smaller than its stride says",
        ));
    }
    out.clear();
    out.reserve(width * rows);
    for row in 0..rows {
        out.extend_from_slice(&source[row * stride..row * stride + width]);
    }
    Ok(())
}

/// Copies a decoded picture of `size` into tight I420 planes.
fn copy_picture(yuv: &impl YUVSource, size: FrameSize, out: &mut I420Image) -> io::Result<()> {
    let (width, height) = yuv.dimensions();
    if (width, height) != (size.width as usize, size.height as usize) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decoded picture differs from the size of its picture stream",
        ));
    }
    let (y_stride, u_stride, v_stride) = yuv.strides();
    let chroma = size.chroma();
    let (chroma_width, chroma_rows) = (chroma.width as usize, chroma.height as usize);
    copy_plane(yuv.y(), y_stride, width, height, &mut out.y)?;
    copy_plane(yuv.u(), u_stride, chroma_width, chroma_rows, &mut out.u)?;
    copy_plane(yuv.v(), v_stride, chroma_width, chroma_rows, &mut out.v)?;
    out.size = size;
    Ok(())
}

impl ScreenDecoder for OpenH264Decoder {
    fn name(&self) -> String {
        OPENH264_NAME.into()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        let size = self.size;
        {
            let decoded = self.decoder.decode(&record.payload).map_err(coder_error)?;
            let Some(yuv) = decoded else {
                return Ok(false);
            };
            copy_picture(&yuv, size, &mut self.planes)?;
        }
        if self.yuv {
            // The copied planes become the output; the output's old buffers
            // serve the next copy.
            let out = self.yuv_out.get_or_insert_with(|| YuvPicture {
                y: Vec::new(),
                y_stride: 0,
                uv: Vec::new(),
                uv_stride: 0,
                chroma: Chroma::I420 {
                    u: Vec::new(),
                    v: Vec::new(),
                },
                matrix: YuvMatrix::Bt709,
            });
            out.y_stride = size.width as usize;
            out.uv_stride = size.chroma().width as usize;
            std::mem::swap(&mut out.y, &mut self.planes.y);
            if let Chroma::I420 { u, v } = &mut out.chroma {
                std::mem::swap(u, &mut self.planes.u);
                std::mem::swap(v, &mut self.planes.v);
            }
            return Ok(true);
        }
        i420_to_bgra(&self.planes, YuvMatrix::Bt709, &mut self.picture)?;
        Ok(true)
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
}

/// Serializes the task tests that load OpenH264 in this test process (one
/// of them checks that switching off unloads the library).
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
#[path = "openh264_codec_tests.rs"]
pub(crate) mod tests;
#[cfg(test)]
#[path = "openh264_yuv_tests.rs"]
mod yuv_tests;
