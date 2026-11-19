//! What the VA-API decoders of the Linux viewer share (BK4's H.264 output
//! path, generalised by BK4b for HEVC and AV1): the read-back of a decoded
//! NV12 surface into the existing plane path (`YuvPicture` with
//! `Chroma::Nv12`, `G/screen_wgpu_yuv.rs`) or into BGRA, the decode config
//! and the context of one geometry, and the `ScreenDecoder` around a codec
//! (`VaVideoDecoder`) with the rule of BK4: a stream outside the screen
//! contract or a driver error locks this decoder for the rest of the
//! process, so the next picture group opens the next decoder of the
//! register (dav1d for AV1). Sources: libva 2.20.0 `va.h`
//! (`vaCreateConfig`, `vaCreateContext`, `vaSyncSurface`).
use std::ffi::c_int;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use super::vaapi_ffi::{
    VaConfigAttrib, VaDisplay, VaId, VA_CONFIG_ATTRIB_RT_FORMAT, VA_ENTRYPOINT_VLD, VA_INVALID_ID,
    VA_PROGRESSIVE, VA_RT_FORMAT_YUV420,
};
use super::vaapi_probe::{VaDevice, VaVideoDecodeCaps};
use super::vaapi_surface as surface;
use crate::screen::core::codec_registry::DecoderFactory;
use crate::screen::core::decoded_frame::DecodedFrame;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::frame_size_caps::check_video_dimensions;
use crate::screen::core::host_backend::{Chroma, ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::{CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::{nv12_to_bgra, BgraImage, FrameSize, Nv12View, YuvMatrix};
use crate::screen::core::wire::invalid;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

/// HEVC/AV1 decoders (factory names) that failed in this process.
static REFUSED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(super) fn refused(name: &str) -> bool {
    REFUSED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|entry| entry == name)
}

/// Review 2026-11-19 (Gemini 8.2): a decoder that showed pictures and then
/// fails gets one new decoder of its factory (the viewer opens one for the
/// next picture group); a failure before the first picture, or the next
/// failure after that one spare, refuses it for the process. Before, one
/// passing bitstream error kept VA-API off until the viewer restarted.
static SPARED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A decoder of `name` failed; `shown`: after a picture it showed.
pub(super) fn refuse_failed(name: &str, shown: bool) {
    if shown {
        let mut spared = SPARED.lock().unwrap_or_else(PoisonError::into_inner);
        if !spared.iter().any(|entry| entry == name) {
            spared.push(name.to_owned());
            return;
        }
    }
    refuse(name);
}

pub(super) fn refuse(name: &str) {
    let mut list = REFUSED.lock().unwrap_or_else(PoisonError::into_inner);
    if !list.iter().any(|entry| entry == name) {
        list.push(name.to_owned());
    }
}

/// The decoded picture of one decoder: NV12 planes read back from the
/// surface, handed out as NV12 or converted to BGRA.
pub(super) struct DecodeOutput {
    size: FrameSize,
    limits: FrameLimits,
    y: Vec<u8>,
    uv: Vec<u8>,
    picture: BgraImage,
    yuv: bool,
    yuv_out: Option<YuvPicture>,
    /// Planes the viewer gave back (`recycle_yuv`, r1): the storage of the
    /// next hand-over, so it needs no new allocation.
    spare: Option<YuvPicture>,
}

impl DecodeOutput {
    pub(super) fn new(size: FrameSize, limits: FrameLimits) -> io::Result<Self> {
        Ok(Self {
            size,
            limits,
            y: Vec::new(),
            uv: Vec::new(),
            picture: BgraImage::new_with_limits(size, limits)?,
            yuv: false,
            yuv_out: None,
            spare: None,
        })
    }

    /// Reads the top left `size` of `target` and publishes it.
    pub(super) fn read(&mut self, display: &VaDisplay, target: VaId) -> io::Result<()> {
        surface::read_nv12(display, target, self.size, &mut self.y, &mut self.uv)?;
        self.publish()
    }

    /// Hands the read-back planes out as NV12 or converts them to BGRA.
    fn publish(&mut self) -> io::Result<()> {
        let width = self.size.width as usize;
        let pairs = self.size.chroma().width as usize * 2;
        if self.yuv {
            let spare = &mut self.spare;
            let out = self.yuv_out.get_or_insert_with(|| {
                spare.take().unwrap_or_else(|| YuvPicture {
                    y: Vec::new(),
                    y_stride: width,
                    uv: Vec::new(),
                    uv_stride: pairs,
                    chroma: Chroma::Nv12,
                    matrix: YuvMatrix::Bt709,
                })
            });
            std::mem::swap(&mut out.y, &mut self.y);
            std::mem::swap(&mut out.uv, &mut self.uv);
            out.y_stride = width;
            out.uv_stride = pairs;
            return Ok(());
        }
        let view =
            Nv12View::new_with_limits(self.size, &self.y, width, &self.uv, pairs, self.limits)?;
        nv12_to_bgra(&view, YuvMatrix::Bt709, &mut self.picture)
    }

    pub(super) fn picture(&self) -> &BgraImage {
        &self.picture
    }

    pub(super) fn prefer_yuv(&mut self, on: bool) {
        self.yuv = on;
        if !on {
            self.yuv_out = None;
            self.spare = None;
        }
    }

    pub(super) fn yuv_picture(&self) -> Option<YuvPicture> {
        self.yuv_out.clone().filter(|_| self.yuv)
    }

    /// Hands the planes of the last picture over instead of copying them
    /// (r1, V1's `ScreenDecoder::take_yuv_picture`); `yuv_picture` is empty
    /// until the next picture.
    pub(super) fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.yuv_out.take().filter(|_| self.yuv)
    }

    /// The planes of a shown picture become the storage of the next one;
    /// planes of another layout are dropped.
    pub(super) fn recycle_yuv(&mut self, planes: YuvPicture) {
        if self.yuv && matches!(planes.chroma, Chroma::Nv12) {
            self.spare = Some(planes);
        }
    }
}

/// A VLD config of `va_profile` for 4:2:0.
pub(super) fn open_decode_config(display: &VaDisplay, va_profile: c_int) -> io::Result<VaId> {
    let mut attribute = VaConfigAttrib {
        kind: VA_CONFIG_ATTRIB_RT_FORMAT,
        value: VA_RT_FORMAT_YUV420,
    };
    let mut config = VA_INVALID_ID;
    // SAFETY: valid display; one attribute that lives through the call.
    let status = unsafe {
        (display.api.create_config)(
            display.handle,
            va_profile,
            VA_ENTRYPOINT_VLD,
            &mut attribute,
            1,
            &mut config,
        )
    };
    display.check(status, "vaCreateConfig (decode)")?;
    Ok(config)
}

/// The decoding context of one geometry: its surfaces and context.
pub(super) struct DecodeContext {
    pub context: VaId,
    pub surfaces: Vec<VaId>,
    pub coded: FrameSize,
}

impl DecodeContext {
    /// `count` NV12 surfaces of `coded` size and a context over them.
    pub(super) fn open(
        display: &VaDisplay,
        config: VaId,
        coded: FrameSize,
        count: usize,
    ) -> io::Result<Self> {
        let mut surfaces = surface::create_nv12_surfaces(display, coded, count)?;
        let mut context = VA_INVALID_ID;
        // SAFETY: config and surfaces belong to this display.
        let status = unsafe {
            (display.api.create_context)(
                display.handle,
                config,
                coded.width as c_int,
                coded.height as c_int,
                VA_PROGRESSIVE,
                surfaces.as_mut_ptr(),
                surfaces.len() as c_int,
                &mut context,
            )
        };
        if let Err(error) = display.check(status, "vaCreateContext (decode)") {
            surface::destroy_surfaces(display, &mut surfaces);
            return Err(error);
        }
        Ok(Self {
            context,
            surfaces,
            coded,
        })
    }

    pub(super) fn close(mut self, display: &VaDisplay) {
        // SAFETY: the context belongs to this display and is idle.
        unsafe { (display.api.destroy_context)(display.handle, self.context) };
        surface::destroy_surfaces(display, &mut self.surfaces);
    }
}

/// The VA objects one codec decoder works with.
pub(super) struct VaDecodeSession {
    pub config: VaId,
    pub context: Option<DecodeContext>,
    pub max: FrameSize,
    pub limits: FrameLimits,
    // Dropped last: config, context and surfaces belong to this display.
    pub display: VaDisplay,
}

impl VaDecodeSession {
    /// The context for `coded` with `count` surfaces, created (anew) when
    /// the geometry changes; `true` when it is new.
    pub(super) fn context_for(&mut self, coded: FrameSize, count: usize) -> io::Result<bool> {
        self.limits.check(coded)?;
        if coded.width > self.max.width || coded.height > self.max.height {
            return Err(invalid("coded surface larger than the VA-API decoder"));
        }
        let current = self.context.as_ref();
        if current.is_some_and(|c| c.coded == coded && c.surfaces.len() == count) {
            return Ok(false);
        }
        if let Some(old) = self.context.take() {
            old.close(&self.display);
        }
        self.context = Some(DecodeContext::open(
            &self.display,
            self.config,
            coded,
            count,
        )?);
        Ok(true)
    }

    pub(super) fn sync(&self, target: VaId) -> io::Result<()> {
        self.display.sync(target, "vaSyncSurface (decode)")
    }
}

impl Drop for VaDecodeSession {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            context.close(&self.display);
        }
        // SAFETY: the config belongs to this display.
        unsafe { (self.display.api.destroy_config)(self.display.handle, self.config) };
    }
}

/// The codec part of a decoder: one record in, the surface of its shown
/// picture out (`None` when the record held none).
pub(super) trait UnitDecoder: Send {
    fn decode_unit(&mut self, va: &mut VaDecodeSession, payload: &[u8])
        -> io::Result<Option<VaId>>;
}

/// The `ScreenDecoder` around a codec part.
pub(super) struct VaVideoDecoder<U: UnitDecoder> {
    name: String,
    codec: CodecKind,
    size: FrameSize,
    limits: FrameLimits,
    unit: U,
    output: DecodeOutput,
    decoded: Option<DecodedFrame>,
    /// It showed a picture (`refuse_failed`).
    shown: bool,
    session: VaDecodeSession,
}

impl<U: UnitDecoder> ScreenDecoder for VaVideoDecoder<U> {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        check_video_dimensions(self.codec, &record.payload, self.size, self.limits)?;
        let decoded = self
            .unit
            .decode_unit(&mut self.session, &record.payload)
            .and_then(|target| match target {
                Some(target) => {
                    self.session.sync(target)?;
                    self.output.read(&self.session.display, target)?;
                    Ok(true)
                }
                None => Ok(false),
            });
        match decoded {
            Ok(shown) => {
                if shown {
                    self.decoded = Some(DecodedFrame::from_record(record));
                    self.shown = true;
                }
                Ok(shown)
            }
            Err(error) => {
                refuse_failed(&self.name, self.shown);
                Err(error)
            }
        }
    }

    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }

    fn picture(&self) -> &BgraImage {
        self.output.picture()
    }

    fn prefer_yuv(&mut self, on: bool) {
        self.output.prefer_yuv(on);
    }

    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.output.yuv_picture()
    }

    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.output.take_yuv_picture()
    }

    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.output.recycle_yuv(planes);
    }

    /// B-36 (r1): the references live in the decoder's own bookkeeping,
    /// which only frames it decoded change; a continuation that names only
    /// a picture the viewer confirmed (so one it decoded) finds it there:
    /// HEVC by its picture order count in the buffer the RPS of every
    /// decoded picture kept (`vaapi_hevc_dpb.rs` `apply_rps`; a missing
    /// used reference is an error, never a wrong picture), AV1 in the slot
    /// of `ref_frame_idx` that only `refresh_frame_flags` of decoded frames
    /// change (`vaapi_av1_decode.rs` `refresh`; an empty slot is an error).
    fn keeps_references(&self) -> bool {
        continues(self.codec)
    }
}

/// Whether the VA-API decoder of `codec` goes on over a continuation (B-36,
/// r1): HEVC and AV1 yes (`keeps_references` above); H.264 no, like every
/// H.264 decoder of se (a gap in `frame_num` is a loss, H.264 8.2.5.2).
pub(super) fn continues(codec: CodecKind) -> bool {
    matches!(codec, CodecKind::Hevc | CodecKind::Av1)
}

/// The VA-API decoder of HEVC Main or AV1 Main on one device.
pub(super) struct VaVideoDecoderFactory {
    pub device: Arc<VaDevice>,
    pub caps: VaVideoDecodeCaps,
}

impl DecoderFactory for VaVideoDecoderFactory {
    fn name(&self) -> String {
        format!(
            "vaapi-{}-{}",
            self.caps.codec.key(),
            self.device.node_name()
        )
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::video(self.caps.codec, false, true).with_max(self.caps.max)]
    }

    fn keeps_references(&self, codec: CodecKind) -> bool {
        codec == self.caps.codec && continues(codec)
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        let name = self.name();
        if refused(&name) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "VA-API decoder failed earlier in this process",
            ));
        }
        if header.codec != self.caps.codec || header.profile.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "VA-API decodes 4:2:0 Main of HEVC and AV1 here",
            ));
        }
        let limits = header
            .frame_contract
            .map_or(FrameLimits::LEGACY, |c| c.limits);
        limits.check(header.size)?;
        let size = header.size;
        if size.width > self.caps.max.width || size.height > self.caps.max.height {
            return Err(invalid("picture larger than the VA-API decoder"));
        }
        let output = DecodeOutput::new(size, limits)?;
        let api = super::vaapi_ffi::load().map_err(io::Error::other)?;
        let display = VaDisplay::open(api, &self.device.node)?;
        let config = open_decode_config(&display, self.caps.va_profile)?;
        let session = VaDecodeSession {
            config,
            context: None,
            max: self.caps.max,
            limits,
            display,
        };
        let decoder: Box<dyn ScreenDecoder> = match self.caps.codec {
            CodecKind::Hevc => Box::new(VaVideoDecoder {
                name,
                codec: CodecKind::Hevc,
                size,
                limits,
                unit: super::vaapi_hevc_decode::HevcUnits::new(size),
                output,
                decoded: None,
                shown: false,
                session,
            }),
            CodecKind::Av1 => Box::new(VaVideoDecoder {
                name,
                codec: CodecKind::Av1,
                size,
                limits,
                unit: super::vaapi_av1_decode::Av1Units::new(size),
                output,
                decoded: None,
                shown: false,
                session,
            }),
            CodecKind::H264 | CodecKind::Tiles => {
                return Err(invalid("VA-API video decoder carries HEVC and AV1 only"))
            }
        };
        Ok(decoder)
    }
}

#[cfg(test)]
#[path = "bildkette_r1_decode_task_tests.rs"]
mod bildkette_r1_decode_task_tests;
