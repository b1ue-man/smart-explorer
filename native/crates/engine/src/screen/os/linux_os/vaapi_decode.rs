//! VA-API H.264 decoder of the Linux viewer (BK4, B-38): the program
//! parses SPS, PPS and slice headers and keeps the reference pictures
//! (`vaapi_h264_syntax.rs`, `vaapi_h264_slice.rs`); the GPU decodes the
//! slices; the NV12 surface is read back into the existing plane path
//! (`YuvPicture` with `Chroma::Nv12`, `G/screen_wgpu_yuv.rs`) or turned
//! into BGRA. A stream outside the screen contract or a driver error marks
//! the device as refused for this process, so the next picture group opens
//! the next decoder of the register (OpenH264 after consent), like the
//! host's `refused` list does for encoders.
use std::collections::BTreeMap;
use std::ffi::c_int;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use super::h264_bits::{nal_type, nal_units, NAL_IDR, NAL_PPS, NAL_SLICE, NAL_SPS};
use super::vaapi_decode_core::DecodeOutput;
use super::vaapi_decode_params::{iq_matrix, picture_params, slice_params};
use super::vaapi_ffi::{
    VaConfigAttrib, VaDisplay, VaId, VA_CONFIG_ATTRIB_RT_FORMAT, VA_ENTRYPOINT_VLD, VA_INVALID_ID,
    VA_IQ_MATRIX_BUFFER, VA_PICTURE_PARAMETER_BUFFER, VA_PROGRESSIVE, VA_RT_FORMAT_YUV420,
    VA_SLICE_DATA_BUFFER, VA_SLICE_PARAMETER_BUFFER,
};
use super::vaapi_h264_dpb::Dpb;
use super::vaapi_h264_slice::{parse_slice, slice_pps_id};
use super::vaapi_h264_syntax::{parse_pps, parse_sps, Pps, Sps};
use super::vaapi_probe::VaDevice;
use super::vaapi_surface as surface;
use super::vaapi_vpp::Buffers;
use crate::screen::core::codec_registry::DecoderFactory;
use crate::screen::core::decoded_frame::{DecodedFrame, FrameIdentities};
use crate::screen::core::decoder_health::{session_epoch, SessionSlots};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{ScreenDecoder, YuvPicture};
use crate::screen::core::negotiate::{CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::wire::invalid;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};

/// The render nodes whose decoder failed (`.0`) and those that had their one
/// spare (`.1`), per viewer session (`decoder_health::session_epoch`, as
/// FFmpeg's given-up stages). Review 2026-11-19 (Gemini 8.2): a decoder that
/// showed pictures and then fails gets one new decoder (the viewer opens one
/// for the next picture group); a failure before the first picture, or the
/// next one after that spare, refuses it. Codex round 4, finding 19: for
/// the session it happened in only; a later session opens VA-API again
/// (before, one bitstream error kept it off until the viewer restarted).
static FAILED: Mutex<SessionSlots<(Vec<PathBuf>, Vec<PathBuf>)>> = Mutex::new(SessionSlots::new());

pub(super) fn refused(node: &Path) -> bool {
    let mut failed = FAILED.lock().unwrap_or_else(PoisonError::into_inner);
    failed
        .get(session_epoch())
        .0
        .iter()
        .any(|entry| entry == node)
}

/// A decoder of `node` failed; `shown`: after a picture it showed.
pub(super) fn refuse_failed(node: &Path, shown: bool) {
    let mut failed = FAILED.lock().unwrap_or_else(PoisonError::into_inner);
    let (refused, spared) = failed.get(session_epoch());
    if shown && !spared.iter().any(|entry| entry == node) {
        spared.push(node.to_path_buf());
        return;
    }
    if !refused.iter().any(|entry| entry == node) {
        refused.push(node.to_path_buf());
    }
}

/// The VA-API decoder of one device.
pub(super) struct VaDecoderFactory {
    pub device: Arc<VaDevice>,
}

impl DecoderFactory for VaDecoderFactory {
    fn name(&self) -> String {
        format!("vaapi-{}", self.device.node_name())
    }

    fn offers(&self) -> Vec<CodecOffer> {
        let Some(caps) = &self.device.decode else {
            return Vec::new();
        };
        caps.profiles
            .iter()
            .map(|profile| CodecOffer::h264(*profile, true).with_max(caps.max))
            .collect()
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        let caps = self.device.decode.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "VA-API device does not decode H.264",
            )
        })?;
        if refused(&self.device.node) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "VA-API decoder failed earlier in this process",
            ));
        }
        if header.codec != CodecKind::H264 {
            return Err(invalid("VA-API decodes H.264 only"));
        }
        let limits = header
            .frame_contract
            .map_or(FrameLimits::LEGACY, |c| c.limits);
        limits.check(header.size)?;
        let size = header.size;
        if size.width > caps.max.width || size.height > caps.max.height {
            return Err(invalid("picture larger than the VA-API decoder"));
        }
        let output = DecodeOutput::new(size, limits)?;
        let api = super::vaapi_ffi::load().map_err(io::Error::other)?;
        let display = VaDisplay::open(api, &self.device.node)?;
        let mut attribute = VaConfigAttrib {
            kind: VA_CONFIG_ATTRIB_RT_FORMAT,
            value: VA_RT_FORMAT_YUV420,
        };
        let mut config = VA_INVALID_ID;
        // SAFETY: valid display; one attribute that lives through the call.
        let status = unsafe {
            (display.api.create_config)(
                display.handle,
                caps.va_profile,
                VA_ENTRYPOINT_VLD,
                &mut attribute,
                1,
                &mut config,
            )
        };
        display.check(status, "vaCreateConfig (decode)")?;
        Ok(Box::new(VaDecoder {
            name: self.name(),
            node: self.device.node.clone(),
            size,
            max: caps.max,
            limits,
            config,
            context: None,
            sps: BTreeMap::new(),
            pps: BTreeMap::new(),
            dpb: Dpb::default(),
            output,
            frames: 0,
            identities: FrameIdentities::default(),
            decoded: None,
            shown: false,
            display,
        }))
    }
}

/// The decoding context of one SPS geometry: its surfaces and context.
struct Context {
    context: VaId,
    surfaces: Vec<VaId>,
    width_mbs: u32,
    height_mbs: u32,
    references: u32,
}

/// One VA-API decoder of a picture group.
struct VaDecoder {
    name: String,
    node: PathBuf,
    size: FrameSize,
    max: FrameSize,
    limits: FrameLimits,
    config: VaId,
    context: Option<Context>,
    sps: BTreeMap<u32, Sps>,
    pps: BTreeMap<u32, Pps>,
    dpb: Dpb,
    /// Read-back and hand-out, shared with HEVC and AV1 (BK4b).
    output: DecodeOutput,
    frames: u64,
    identities: FrameIdentities,
    decoded: Option<DecodedFrame>,
    /// It showed a picture (`refuse_failed`).
    shown: bool,
    // Dropped last: config, context and surfaces belong to this display.
    display: VaDisplay,
}

impl VaDecoder {
    /// The context for `sps`, created (anew) when its geometry changes.
    fn context_for(&mut self, sps: &Sps) -> io::Result<()> {
        let references = sps.max_num_ref_frames.max(1);
        if self.context.as_ref().is_some_and(|c| {
            c.width_mbs == sps.width_mbs
                && c.height_mbs == sps.height_mbs
                && c.references == references
        }) {
            return Ok(());
        }
        self.close_context();
        let coded = FrameSize::new(sps.width_mbs * 16, sps.height_mbs * 16);
        if coded.width < self.size.width || coded.height < self.size.height {
            return Err(invalid("H.264 SPS smaller than its picture group"));
        }
        self.limits.check(coded)?;
        if coded.width > self.max.width || coded.height > self.max.height {
            return Err(invalid(
                "H.264 coded surface larger than the VA-API decoder",
            ));
        }
        // References, the current picture and one spare.
        let count = references as usize + 2;
        let mut surfaces = surface::create_nv12_surfaces(&self.display, coded, count)?;
        let mut context = VA_INVALID_ID;
        // SAFETY: config and surfaces belong to this display.
        let status = unsafe {
            (self.display.api.create_context)(
                self.display.handle,
                self.config,
                coded.width as c_int,
                coded.height as c_int,
                VA_PROGRESSIVE,
                surfaces.as_mut_ptr(),
                surfaces.len() as c_int,
                &mut context,
            )
        };
        if let Err(error) = self.display.check(status, "vaCreateContext (decode)") {
            surface::destroy_surfaces(&self.display, &mut surfaces);
            return Err(error);
        }
        self.context = Some(Context {
            context,
            surfaces,
            width_mbs: sps.width_mbs,
            height_mbs: sps.height_mbs,
            references,
        });
        self.dpb = Dpb::default();
        Ok(())
    }

    fn close_context(&mut self) {
        if let Some(mut context) = self.context.take() {
            // SAFETY: the context belongs to this display and is idle.
            unsafe { (self.display.api.destroy_context)(self.display.handle, context.context) };
            surface::destroy_surfaces(&self.display, &mut context.surfaces);
        }
    }

    /// Decodes one access unit; `false` when it held no picture.
    fn decode_unit(&mut self, payload: &[u8]) -> io::Result<bool> {
        let units = nal_units(payload);
        let mut slices = Vec::new();
        for unit in units {
            match nal_type(unit) {
                NAL_SPS => {
                    let sps = parse_sps(unit)?;
                    self.sps.insert(sps.id, sps);
                }
                NAL_PPS => {
                    let pps = parse_pps(unit, |id| self.sps.get(&id).cloned())?;
                    self.pps.insert(pps.id, pps);
                }
                NAL_SLICE | NAL_IDR => slices.push(unit),
                14 | 15 | 20 => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "H.264 SVC/MVC is outside the screen contract",
                    ))
                }
                _ => {}
            }
        }
        let Some(first) = slices.first() else {
            return Ok(false);
        };
        let pps = self
            .pps
            .get(&slice_pps_id(first)?)
            .cloned()
            .ok_or_else(|| invalid("H.264 slice names an unknown PPS"))?;
        let sps = self
            .sps
            .get(&pps.sps_id)
            .cloned()
            .ok_or_else(|| invalid("H.264 PPS names an unknown SPS"))?;
        let header = parse_slice(first, &sps, &pps)?;
        if header.idr {
            self.context_for(&sps)?;
        } else if self.context.as_ref().is_none_or(|c| {
            c.width_mbs != sps.width_mbs
                || c.height_mbs != sps.height_mbs
                || sps.max_num_ref_frames.max(1) > c.references
        }) {
            return Err(invalid("H.264 non-IDR picture without matching VA context"));
        }
        let total_mbs = sps.width_mbs * sps.height_mbs;
        if header.first_mb != 0 || slices.len() > total_mbs as usize {
            return Err(invalid("H.264 slice layout invalid"));
        }
        self.dpb.check_gap(&header, &sps)?;
        let poc = self.dpb.poc(&header, &sps)?;
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| invalid("no VA context"))?;
        let target = *context
            .surfaces
            .iter()
            .find(|candidate| !self.dpb.refs.iter().any(|r| r.surface == **candidate))
            .ok_or_else(|| invalid("no free VA surface for the picture"))?;
        let mut buffers = Buffers::new(&self.display, context.context);
        let picture = picture_params(&sps, &pps, &header, &self.dpb, target, poc);
        buffers.add(VA_PICTURE_PARAMETER_BUFFER, &picture)?;
        buffers.add(VA_IQ_MATRIX_BUFFER, &iq_matrix(&sps, &pps))?;
        let mut last_mb = 0u32;
        for (index, unit) in slices.iter().enumerate() {
            let parsed;
            let slice = if index == 0 {
                &header
            } else {
                parsed = parse_slice(unit, &sps, &pps)?;
                if parsed.first_mb <= last_mb || parsed.first_mb >= total_mbs {
                    return Err(invalid("H.264 slice macroblock order invalid"));
                }
                last_mb = parsed.first_mb;
                &parsed
            };
            if slice.pps_id != pps.id
                || slice.frame_num != header.frame_num
                || slice.idr != header.idr
                || (slice.nal_ref_idc != 0) != (header.nal_ref_idc != 0)
            {
                return Err(invalid("H.264 slices of one picture disagree"));
            }
            let list0 = if slice.slice_type == 0 {
                self.dpb.list0(slice, &sps)?
            } else {
                Vec::new()
            };
            let parameters = slice_params(slice, &list0, unit.len())?;
            buffers.add(VA_SLICE_PARAMETER_BUFFER, &parameters)?;
            buffers.add_bytes(VA_SLICE_DATA_BUFFER, unit)?;
        }
        buffers.submit(target)?;
        drop(buffers);
        self.display.sync(target, "vaSyncSurface (decode)")?;
        self.output.read(&self.display, target)?;
        self.dpb.mark(&header, &sps, target, poc)?;
        Ok(true)
    }
}

impl ScreenDecoder for VaDecoder {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        crate::screen::core::frame_size_caps::check_avc_dimensions(
            &record.payload,
            self.size,
            self.limits,
        )?;
        self.frames = self
            .frames
            .checked_add(1)
            .ok_or_else(|| invalid("VA-API frame counter overflow"))?;
        self.identities
            .insert(self.frames, DecodedFrame::from_record(record))?;
        match self.decode_unit(&record.payload) {
            Ok(true) => {
                let identity = self.identities.take(self.frames)?;
                self.decoded = Some(identity);
                self.shown = true;
                Ok(true)
            }
            Ok(false) => {
                let _ = self.identities.take(self.frames);
                Ok(false)
            }
            Err(error) => {
                refuse_failed(&self.node, self.shown);
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

    /// r1: the planes go to the viewer without a copy.
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.output.take_yuv_picture()
    }

    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.output.recycle_yuv(planes);
    }

    // `keeps_references` stays `false` for H.264 (B-36, r1): a gap in
    // `frame_num` without `gaps_in_frame_num_value_allowed_flag` is a loss
    // of pictures (H.264 8.2.5.2), as for OpenH264 and Media Foundation.
}

impl Drop for VaDecoder {
    fn drop(&mut self) {
        self.close_context();
        // SAFETY: the config belongs to this display.
        unsafe { (self.display.api.destroy_config)(self.display.handle, self.config) };
    }
}
