//! Test-only v0.5.184 coders, preserved verbatim between the frozen markers.
//! No candidate factory, converter or decoder is substituted for this arm.
use super::{coder_error, load, OpenH264Store, OPENH264_NAME};
use crate::screen::core::{
    host_backend::{Chroma, EncodedFrame, ScreenDecoder, ScreenEncoder, YuvPicture},
    pixel_convert::YuvMatrix,
    pixel_convert::{BgraImage, FrameSize, I420Image},
    quality_acceptance_baseline as pixels,
    wire_binary::FrameRecord,
};
use openh264::{
    decoder::{Decoder, DecoderConfig},
    encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, Profile,
        RateControlMode, UsageType, VuiConfig,
    },
    formats::{YUVSlices, YUVSource},
    OpenH264API,
};
use std::{
    io,
    os::raw::{c_int, c_void},
};
const MIN_BITRATE_BPS: u64 = 150_000;
const MAX_BITRATE_BPS: u64 = 240_000_000;
const ENCODER_OPTION_BITRATE: c_int = 5;
const ENCODER_OPTION_MAX_BITRATE: c_int = 6;
const SPATIAL_LAYER_0: c_int = 0;
const SPATIAL_LAYER_ALL: c_int = 4;
#[repr(C)]
struct BitrateInfo {
    layer: c_int,
    bitrate: c_int,
}
fn bgra_to_i420(
    view: &crate::screen::core::pixel_convert::BgraView<'_>,
    _: YuvMatrix,
) -> io::Result<I420Image> {
    pixels::bgra_to_i420(view, pixels::YuvMatrix::Bt709)
}
fn i420_to_bgra(source: &I420Image, _: YuvMatrix, output: &mut BgraImage) -> io::Result<()> {
    pixels::i420_to_bgra(source, pixels::YuvMatrix::Bt709, output)
}
// BEGIN_FROZEN_ENCODER
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

// END_FROZEN_ENCODER
// BEGIN_FROZEN_DECODER
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

// END_FROZEN_DECODER
pub(super) struct Baseline {
    pub(super) encoder: Box<dyn ScreenEncoder>,
    decoder: OpenH264Decoder,
}
pub(super) fn open(store: &OpenH264Store, size: FrameSize, fps: u32) -> io::Result<Baseline> {
    let encoder = OpenH264Encoder {
        api: Some(load(store)?),
        encoder: None,
        size,
        fps: fps.max(1),
        bitrate: 0,
    };
    let decoder = OpenH264Decoder {
        decoder: Decoder::with_api_config(load(store)?, DecoderConfig::new())
            .map_err(coder_error)?,
        size,
        planes: I420Image {
            size,
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
        },
        picture: BgraImage::new(size)?,
        yuv: false,
        yuv_out: None,
    };
    Ok(Baseline {
        encoder: Box::new(encoder),
        decoder,
    })
}
impl Baseline {
    pub(super) fn decode(&mut self, record: &FrameRecord) -> io::Result<Option<&BgraImage>> {
        if !self.decoder.decode(record)? {
            return Ok(None);
        }
        let mut remaining: c_int = 0;
        // SAFETY: the historical decoder is initialized; get_option writes one
        // c_int while this exclusive borrow keeps its native instance alive.
        let status = unsafe {
            self.decoder.decoder.raw_api().get_option(
                openh264_sys2::DECODER_OPTION_NUM_OF_FRAMES_REMAINING_IN_BUFFER,
                (&mut remaining as *mut c_int).cast(),
            )
        };
        if status != 0 || remaining != 0 {
            return Err(io::Error::other(
                "baseline output cannot be tied to exactly one synchronous capture",
            ));
        }
        Ok(Some(self.decoder.picture()))
    }
}
