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
use std::os::raw::c_int;
use std::sync::Arc;

use openh264::decoder::{Decoder, DecoderConfig};
use openh264::OpenH264API;
use openh264_sys2::{SBufferInfo, DECODER_OPTION_NUM_OF_FRAMES_REMAINING_IN_BUFFER, videoFormatI420};

use super::codec_inventory::openh264_decoder_offers;
pub(crate) use super::openh264_encode::OpenH264EncoderFactory;
use super::openh264_install::OpenH264Store;
use crate::screen::core::codec_registry::{
    DecoderFactory, EncoderFactory, OpenH264Report, OpenH264State,
};
use crate::screen::core::decoded_frame::{DecodedFrame, FrameIdentities};
use crate::screen::core::host_backend::{
    Chroma, ScreenDecoder, YuvPicture,
};
use crate::screen::core::negotiate::CodecOffer;
#[cfg(test)]
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::pixel_convert::{
    i420_to_bgra, BgraImage, FrameSize, I420Image, YuvMatrix,
};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

/// Name in the statistics and in `se screen codec`.
pub(crate) const OPENH264_NAME: &str = "openh264";
pub(super) fn coder_error(error: openh264::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// Loads the library of `store` after its pin check.
pub(super) fn load(store: &OpenH264Store) -> io::Result<OpenH264API> {
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
        let limits = header.frame_contract.map_or(crate::screen::core::frame_budget::FrameLimits::LEGACY, |c| c.limits);
        limits.check(header.size)?;
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
            picture: BgraImage::new_with_limits(header.size, limits)?,
            limits,
            yuv: false,
            yuv_out: None,
            frames: 0,
            identities: FrameIdentities::default(),
            decoded: None,
        }))
    }
}

/// One OpenH264 decoder of a picture group of `size`.
struct OpenH264Decoder {
    decoder: Decoder,
    size: FrameSize,
    limits: crate::screen::core::frame_budget::FrameLimits,
    planes: I420Image,
    picture: BgraImage,
    /// `prefer_yuv(true)`: hand the planes out instead of converting.
    yuv: bool,
    yuv_out: Option<YuvPicture>,
    frames: u64,
    identities: FrameIdentities,
    decoded: Option<DecodedFrame>,
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

/// Copies a geometry-validated decoded picture into tight owned I420 planes.
fn copy_picture(planes: (&[u8], &[u8], &[u8]), strides: (usize, usize, usize), size: FrameSize, out: &mut I420Image) -> io::Result<()> {
    let (width, height) = (size.width as usize, size.height as usize);
    let (y_stride, u_stride, v_stride) = strides;
    let chroma = size.chroma();
    let (chroma_width, chroma_rows) = (chroma.width as usize, chroma.height as usize);
    copy_plane(planes.0, y_stride, width, height, &mut out.y)?;
    copy_plane(planes.1, u_stride, chroma_width, chroma_rows, &mut out.u)?;
    copy_plane(planes.2, v_stride, chroma_width, chroma_rows, &mut out.v)?;
    out.size = size;
    Ok(())
}

impl ScreenDecoder for OpenH264Decoder {
    fn name(&self) -> String {
        OPENH264_NAME.into()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        let size = self.size;
        crate::screen::core::frame_size_caps::check_avc_dimensions(&record.payload,size,self.limits)?;
        self.frames = self.frames.checked_add(1).ok_or_else(|| io::Error::other("OpenH264 timestamp overflow"))?;
        self.identities.insert(self.frames, DecodedFrame::from_record(record))?;
        let mut dst = [std::ptr::null_mut(); 3];
        let mut buffer = SBufferInfo::default();
        // The pinned 0.9.9 safe wrapper does not set uiInBsTimeStamp and its
        // DecodedYUV::timestamp reads the input field. Preserve the output field
        // ourselves through the existing raw API; no decoder settings change.
        let api = unsafe { self.decoder.raw_api() };
        buffer.uiInBsTimeStamp = self.frames;
        // SAFETY: input bytes and writable output structs live through the call.
        let status = unsafe { api.decode_frame_no_delay(record.payload.as_ptr(),
            record.payload.len() as c_int, dst.as_mut_ptr(), &mut buffer) };
        if status != 0 { return Err(io::Error::other(format!("OpenH264 decode failed: {status}"))); }
        if buffer.iBufferStatus == 0 {
            // Preserve Decoder::decode's existing automatic flush behavior.
            let mut remaining: c_int = 0;
            if unsafe { api.get_option(DECODER_OPTION_NUM_OF_FRAMES_REMAINING_IN_BUFFER, (&mut remaining as *mut c_int).cast()) } != 0 {
                return Err(io::Error::other("OpenH264 buffered-frame query failed"));
            }
            if remaining == 0 { return Ok(false); }
            buffer = Default::default();
            if unsafe { api.flush_frame(dst.as_mut_ptr(), &mut buffer) } != 0 || buffer.iBufferStatus != 1 {
                return Err(io::Error::other("OpenH264 buffered-frame flush failed"));
            }
        }
        if buffer.iBufferStatus != 1 { return Err(io::Error::other("invalid OpenH264 output status")); }
        let identity = self.identities.take(buffer.uiOutYuvTimeStamp)?;
        // SAFETY: a ready I420 output describes the decoder-owned plane buffers.
        let info = unsafe { buffer.UsrData.sSystemBuffer };
        if info.iWidth != size.width as c_int || info.iHeight != size.height as c_int
            || info.iFormat != videoFormatI420 || info.iStride[0] < info.iWidth
            || info.iStride[1] < size.chroma().width as c_int || dst.iter().any(|p| p.is_null()) {
            return Err(io::Error::other("OpenH264 output contradicts its I420 stream geometry"));
        }
        let strides = (info.iStride[0] as usize, info.iStride[1] as usize, info.iStride[1] as usize);
        let length = |stride: usize, rows: usize| stride.checked_mul(rows).filter(|n| *n <= isize::MAX as usize)
            .ok_or_else(|| io::Error::other("OpenH264 plane size overflows address space"));
        let lengths = (length(strides.0, size.height as usize)?, length(strides.1, size.chroma().height as usize)?);
        // SAFETY: geometry/positive strides were checked; OpenH264 owns these
        // readable planes until its next call. copy_picture owns the result.
        let slices = unsafe { (std::slice::from_raw_parts(dst[0], lengths.0),
            std::slice::from_raw_parts(dst[1], lengths.1), std::slice::from_raw_parts(dst[2], lengths.1)) };
        copy_picture(slices, strides, size, &mut self.planes)?;
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
            self.decoded = Some(identity);
            return Ok(true);
        }
        i420_to_bgra(&self.planes, YuvMatrix::Bt709, &mut self.picture)?;
        self.decoded = Some(identity);
        Ok(true)
    }

    fn decoded_frame(&self) -> Option<DecodedFrame> { self.decoded }

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
#[cfg(test)]
#[path = "quality_acceptance_baseline.rs"]
mod quality_baseline;
#[cfg(test)]
#[path = "quality_acceptance_provenance.rs"]
mod quality_provenance;
#[cfg(test)]
#[path = "batches_quality_acceptance_task_tests.rs"]
mod quality_acceptance_tests;
