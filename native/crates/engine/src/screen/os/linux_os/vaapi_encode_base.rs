//! The part every VA-API encoding session shares (BK4, moved here by BK4b
//! so H.264, HEVC and AV1 use one layer): the config with CBR and the
//! packed headers the codec asks for, the surfaces (input, reconstructed
//! pictures), the coded buffer, the video processor that converts the
//! captured BGRA on the GPU (B-18), the DMA-BUF import of BK8b (B-15) and
//! the read-back of the coded segments. Sources: libva 2.20.0 `va.h`
//! (`vaCreateConfig`, `vaCreateContext`, `vaCreateBuffer`, `vaSyncSurface`,
//! `vaMapBuffer`, `VACodedBufferSegment`, `VAEncMiscParameterBuffer`,
//! `VAEncPackedHeaderParameterBuffer`). Merge W2: the fastest quality level
//! the driver offers (`VAConfigAttribEncQualityRange`,
//! `VAEncMiscParameterTypeQualityLevel`) with every sequence, the same
//! low-latency choice as NVENC's P1/"ultra low latency"; a failed
//! conversion of a DMA-BUF is marked (`ConversionFailed`). Plan 2026-11-17
//! P9 with the rule „Wahl gilt“ of 2026-11-18: „Scharf (GPU)“ opens the
//! config with quality-defined VBR (`VA_RC_QVBR`, `quality_factor` = the
//! session's quality target, the rate as its peak) where the driver lists it
//! (`VAConfigAttribRateControl`); a driver without it refuses the session
//! (`SHARP_UNSUPPORTED`) instead of encoding CBR silently.
use std::ffi::{c_int, c_uint, c_void};
use std::io;

use super::hw_dmabuf_picture::ConversionFailed;
use super::hw_encode_common::{HwInput, RateParams};
use super::vaapi_dmabuf::va_fourcc_for;
use super::vaapi_ffi::{
    VaCodedBufferSegment, VaConfigAttrib, VaDisplay, VaId, VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
    VA_CONFIG_ATTRIB_RATE_CONTROL, VA_CONFIG_ATTRIB_RT_FORMAT, VA_ENC_CODED_BUFFER,
    VA_ENC_MISC_PARAMETER_BUFFER, VA_ENC_PACKED_HEADER_DATA_BUFFER,
    VA_ENC_PACKED_HEADER_PARAMETER_BUFFER, VA_INVALID_ID, VA_PROGRESSIVE, VA_RC_CBR, VA_RC_QVBR,
    VA_RT_FORMAT_YUV420,
};
use super::vaapi_h264_abi::{
    VaEncMisc, VaEncMiscQualityLevel, VaEncMiscRateControl, VaEncPackedHeaderParameter,
    VA_ENC_MISC_FRAME_RATE, VA_ENC_MISC_HRD, VA_ENC_MISC_QUALITY_LEVEL, VA_ENC_MISC_RATE_CONTROL,
};
use super::vaapi_h264_headers::misc_params;
use super::vaapi_surface as surface;
use super::vaapi_vpp::{self as vpp, Buffers, Converter, DmaBufFrame};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;

pub(super) type Deliver<'a> = &'a mut dyn FnMut(&[&[u8]]) -> io::Result<()>;

/// The error text of a VA-API session that cannot carry „Scharf (GPU)“.
pub(super) const SHARP_UNSUPPORTED: &str =
    "sharp_quality_unsupported: the VA-API driver lists no quality-defined VBR (VA_RC_QVBR) \
for this profile";

/// What a codec session asks of the shared layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EncodeSetup {
    pub va_profile: c_int,
    pub entrypoint: c_int,
    /// `VA_ENC_PACKED_HEADER_*` bits the session will supply (0: none).
    pub packed_headers: u32,
    /// The picture size (top left part of every surface).
    pub size: FrameSize,
    /// The surface size (the codec's block alignment).
    pub surface: FrameSize,
    /// Reconstructed surfaces besides the input.
    pub recon: usize,
    pub video_proc: bool,
    /// The constant quality of sharp mode (1–51, P9); `None`: CBR.
    pub quality: Option<u8>,
}

/// One VA encoding context with its surfaces; dropped with the display.
pub(super) struct VaEncodeBase {
    pub config: VaId,
    pub context: VaId,
    /// `[input, reconstructed…]`.
    pub surfaces: Vec<VaId>,
    pub coded: VaId,
    pub size: FrameSize,
    converter: Option<Converter>,
    /// The fastest quality level of the driver, `None` with a single one.
    quality_level: Option<u32>,
    /// The constant quality of sharp mode (QVBR), `None`: CBR.
    quality: Option<u8>,
    // Dropped last: every id above belongs to this display.
    pub display: VaDisplay,
}

impl VaEncodeBase {
    pub(super) fn open(display: VaDisplay, setup: &EncodeSetup) -> io::Result<Self> {
        let attribute = |kind, value| VaConfigAttrib { kind, value };
        let listed = match setup.quality {
            Some(_) => super::vaapi_probe::attributes(
                &display,
                setup.va_profile,
                setup.entrypoint,
                &[VA_CONFIG_ATTRIB_RATE_CONTROL],
            )
            .first()
            .copied()
            .unwrap_or(super::vaapi_ffi::VA_ATTRIB_NOT_SUPPORTED),
            None => 0,
        };
        let mut attribs = vec![
            attribute(VA_CONFIG_ATTRIB_RT_FORMAT, VA_RT_FORMAT_YUV420),
            attribute(
                VA_CONFIG_ATTRIB_RATE_CONTROL,
                rate_control(setup.quality, listed)?,
            ),
        ];
        if setup.packed_headers != 0 {
            attribs.push(attribute(
                VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
                setup.packed_headers,
            ));
        }
        let mut config = VA_INVALID_ID;
        // SAFETY: valid display; the attribute list lives through the call.
        let status = unsafe {
            (display.api.create_config)(
                display.handle,
                setup.va_profile,
                setup.entrypoint,
                attribs.as_mut_ptr(),
                attribs.len() as c_int,
                &mut config,
            )
        };
        display.check(status, "vaCreateConfig (encode)")?;
        let range = super::vaapi_probe::attributes(
            &display,
            setup.va_profile,
            setup.entrypoint,
            &[super::vaapi_ffi::VA_CONFIG_ATTRIB_ENC_QUALITY_RANGE],
        );
        let mut base = Self {
            config,
            context: VA_INVALID_ID,
            surfaces: Vec::new(),
            coded: VA_INVALID_ID,
            size: setup.size,
            converter: None,
            quality_level: range.first().copied().and_then(fastest_quality_level),
            quality: setup.quality,
            display,
        };
        let coded = setup.surface;
        base.surfaces = surface::create_nv12_surfaces(&base.display, coded, 1 + setup.recon)?;
        // SAFETY: config and surfaces belong to this display.
        let status = unsafe {
            (base.display.api.create_context)(
                base.display.handle,
                config,
                coded.width as c_int,
                coded.height as c_int,
                VA_PROGRESSIVE,
                base.surfaces[1..].as_mut_ptr(),
                setup.recon as c_int,
                &mut base.context,
            )
        };
        base.display.check(status, "vaCreateContext (encode)")?;
        // The coded buffer holds an uncompressed picture and a margin.
        let bytes = coded.pixels().saturating_mul(3) / 2 + 64 * 1024;
        let bytes = c_uint::try_from(bytes).map_err(|_| invalid("VA coded buffer too large"))?;
        // SAFETY: valid context; no initial data.
        let status = unsafe {
            (base.display.api.create_buffer)(
                base.display.handle,
                base.context,
                VA_ENC_CODED_BUFFER,
                bytes,
                1,
                std::ptr::null_mut(),
                &mut base.coded,
            )
        };
        base.display.check(status, "vaCreateBuffer (coded)")?;
        if setup.video_proc {
            // B-18; without it the CPU converts.
            let input = base.surfaces[0];
            let opened = Converter::open(&base.display, setup.size, coded, input);
            base.converter = opened.ok();
        }
        Ok(base)
    }

    pub(super) fn input(&self) -> VaId {
        self.surfaces[0]
    }

    /// Whether the session takes BGRA (the video processor converts it).
    pub(super) fn wants_bgra(&self) -> bool {
        self.converter.is_some()
    }

    /// Puts `input` into the input surface (BGRA through the video
    /// processor, I420 from the CPU; `Prepared` is already there). The
    /// video processor of the BGRA input is queued without a wait of its own,
    /// so its time stays in the station `encode` (`convert_clock`).
    pub(super) fn upload(&mut self, input: HwInput<'_>) -> io::Result<()> {
        let (target, size) = (self.surfaces[0], self.size);
        match (input, &self.converter) {
            (HwInput::Bgra(view), Some(converter)) => {
                surface::upload_bgra(&self.display, converter.rgb, converter.fourcc, &view)?;
                vpp::convert(
                    &self.display,
                    converter.context,
                    converter.rgb,
                    size,
                    target,
                    size,
                )
            }
            (HwInput::I420(yuv), _) => surface::upload_i420(&self.display, target, yuv),
            // `import_dmabuf` filled the input surface (BK8b).
            (HwInput::Prepared, _) => Ok(()),
            (HwInput::Bgra(_), None) => Err(invalid("VA encoder without video processor got BGRA")),
        }
    }

    /// BK8b: imports the DMA-BUF (`vaapi_vpp::import_dmabuf`, no copy), lets
    /// the video processor convert and scale it into the input surface and
    /// waits for that (`vaSyncSurface` on the input), so the PipeWire buffer
    /// may go back right after; the import surface is destroyed here.
    pub(super) fn import_dmabuf(&mut self, frame: &DmaBufFrame) -> io::Result<()> {
        // Station `convert`: import, video processor and the wait for it.
        let _convert = crate::screen::core::convert_clock::Timed::start();
        let converter = self.converter.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "no VA video processor for a DMA-BUF",
            )
        })?;
        let fourcc = va_fourcc_for(frame.drm_format).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "DMA-BUF format without a VA layout",
            )
        })?;
        let (context, target, size) = (converter.context, self.surfaces[0], self.size);
        let mut source = [vpp::import_dmabuf(&self.display, frame, fourcc)?];
        let converted = vpp::convert(&self.display, context, source[0], frame.size, target, size)
            .and_then(|()| self.sync(target, "vaSyncSurface (DMA-BUF conversion)"));
        surface::destroy_surfaces(&self.display, &mut source);
        // The import worked: a failure from here on is the conversion's.
        converted.map_err(|error| io::Error::new(error.kind(), ConversionFailed(error.to_string())))
    }

    pub(super) fn sync(&self, surface: VaId, what: &str) -> io::Result<()> {
        self.display.sync(surface, what)
    }

    /// Adds a packed header (`VAEncPackedHeaderParameterBuffer` and its
    /// data) of `kind` whose bytes carry emulation prevention already.
    pub(super) fn add_packed(
        buffers: &mut Buffers<'_>,
        kind: u32,
        bytes: &[u8],
        bit_length: u32,
    ) -> io::Result<()> {
        let header = VaEncPackedHeaderParameter {
            kind,
            bit_length,
            has_emulation_bytes: 1,
            ..Default::default()
        };
        buffers.add(VA_ENC_PACKED_HEADER_PARAMETER_BUFFER, &header)?;
        buffers.add_bytes(VA_ENC_PACKED_HEADER_DATA_BUFFER, bytes)?;
        Ok(())
    }

    /// Adds the rate control, HRD and frame rate misc parameters of `rate`
    /// (B-21: CBR, buffer of one frame; `reset` on a change at run time;
    /// sharp mode: the quality factor of QVBR, P9).
    pub(super) fn add_rate(
        &self,
        buffers: &mut Buffers<'_>,
        rate: &RateParams,
        reset: bool,
    ) -> io::Result<()> {
        let (control, hrd, frame_rate) = misc_params(rate, reset);
        let control = with_quality(control, self.quality);
        let misc = VA_ENC_MISC_PARAMETER_BUFFER;
        let control = VaEncMisc {
            kind: VA_ENC_MISC_RATE_CONTROL,
            data: control,
        };
        let hrd = VaEncMisc {
            kind: VA_ENC_MISC_HRD,
            data: hrd,
        };
        let frame_rate = VaEncMisc {
            kind: VA_ENC_MISC_FRAME_RATE,
            data: frame_rate,
        };
        buffers.add(misc, &control)?;
        buffers.add(misc, &hrd)?;
        buffers.add(misc, &frame_rate)?;
        Ok(())
    }

    /// Adds the fastest quality level of the driver (merge W2); called with
    /// every sequence, the level holds for the coded sequence (va.h).
    pub(super) fn add_quality(&self, buffers: &mut Buffers<'_>) -> io::Result<()> {
        let Some(quality_level) = self.quality_level else {
            return Ok(());
        };
        let quality = VaEncMisc {
            kind: VA_ENC_MISC_QUALITY_LEVEL,
            data: VaEncMiscQualityLevel {
                quality_level,
                ..Default::default()
            },
        };
        buffers.add(VA_ENC_MISC_PARAMETER_BUFFER, &quality)?;
        Ok(())
    }

    /// Waits for the picture encoded from the input surface and hands the
    /// coded segments to `deliver`.
    pub(super) fn finish(&self, deliver: Deliver<'_>) -> io::Result<()> {
        self.sync(self.surfaces[0], "vaSyncSurface (encode)")?;
        self.read_coded(deliver)
    }

    /// Maps the coded buffer and hands its segments to `deliver`.
    fn read_coded(&self, deliver: Deliver<'_>) -> io::Result<()> {
        let display = &self.display;
        let mut first: *mut c_void = std::ptr::null_mut();
        // SAFETY: the coded buffer belongs to this display; the picture is synced.
        let status = unsafe { (display.api.map_buffer)(display.handle, self.coded, &mut first) };
        display.check(status, "vaMapBuffer (coded)")?;
        let mut segments: Vec<&[u8]> = Vec::new();
        let mut next = first.cast::<VaCodedBufferSegment>();
        let mut layout_error = None;
        let mut steps = 0usize;
        while !next.is_null() && steps < 64 {
            steps += 1;
            // SAFETY: the mapped coded buffer is a list of segments, each
            // valid until vaUnmapBuffer below (va.h, vaMapBuffer).
            let segment = unsafe { &*next };
            if segment.status & 0x8000 != 0 {
                layout_error = Some(invalid("VA coded segment marked corrupt"));
                break;
            }
            if segment.bit_offset != 0 {
                layout_error = Some(invalid("VA coded segment starts inside a byte"));
                break;
            }
            if segment.size != 0 && !segment.buf.is_null() {
                let size = segment.size as usize;
                // SAFETY: `buf` holds `size` bytes while the buffer is mapped.
                segments.push(unsafe { std::slice::from_raw_parts(segment.buf.cast(), size) });
            }
            next = segment.next.cast();
        }
        let result = match layout_error {
            Some(error) => Err(error),
            None => deliver(&segments),
        };
        // SAFETY: the coded buffer was mapped above.
        unsafe { (display.api.unmap_buffer)(display.handle, self.coded) };
        result
    }
}

/// The rate control of the config: CBR, or QVBR for sharp mode (`quality`)
/// when the driver lists it in `listed` (`VAConfigAttribRateControl`);
/// otherwise the session is refused (`SHARP_UNSUPPORTED`).
pub(super) fn rate_control(quality: Option<u8>, listed: u32) -> io::Result<u32> {
    let qvbr = listed != super::vaapi_ffi::VA_ATTRIB_NOT_SUPPORTED && listed & VA_RC_QVBR != 0;
    match (quality, qvbr) {
        (None, _) => Ok(VA_RC_CBR),
        (Some(_), true) => Ok(VA_RC_QVBR),
        (Some(_), false) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            SHARP_UNSUPPORTED,
        )),
    }
}

/// `control` with the quality factor of QVBR (va.h `quality_factor`, 1–51,
/// lower is better) for sharp mode; unchanged for CBR.
pub(super) fn with_quality(
    mut control: VaEncMiscRateControl,
    quality: Option<u8>,
) -> VaEncMiscRateControl {
    if let Some(quality) = quality {
        control.quality_factor = u32::from(quality.clamp(1, 51));
    }
    control
}

/// The fastest of `range` quality levels (`VAConfigAttribEncQualityRange`):
/// the largest value, as "a value of 1 represents the highest quality" and
/// higher quality means lower speed (va.h 2.20.0 lines 2700-2706); `None`
/// when the driver has one level or does not report the attribute.
pub(super) fn fastest_quality_level(range: u32) -> Option<u32> {
    (range != super::vaapi_ffi::VA_ATTRIB_NOT_SUPPORTED && range > 1).then_some(range)
}

impl Drop for VaEncodeBase {
    fn drop(&mut self) {
        if let Some(converter) = self.converter.take() {
            converter.close(&self.display);
        }
        // SAFETY: every id belongs to this display; invalid ids are skipped.
        unsafe {
            if self.coded != VA_INVALID_ID {
                (self.display.api.destroy_buffer)(self.display.handle, self.coded);
            }
            if self.context != VA_INVALID_ID {
                (self.display.api.destroy_context)(self.display.handle, self.context);
            }
        }
        surface::destroy_surfaces(&self.display, &mut self.surfaces);
        // SAFETY: the config belongs to this display.
        unsafe { (self.display.api.destroy_config)(self.display.handle, self.config) };
    }
}

#[cfg(test)]
#[path = "wahlgilt_vaapi_task_tests.rs"]
mod wahlgilt_vaapi_task_tests;
