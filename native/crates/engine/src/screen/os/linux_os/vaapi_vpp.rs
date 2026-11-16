//! Parameter buffers of one VA picture, the VA video processor (B-18:
//! BGRA → NV12 in BT.709 limited range on the GPU) and the DMA-BUF import
//! for BK8b (B-15). Sources: libva 2.20.0 `va.h` (`vaCreateBuffer`,
//! `vaBeginPicture`, `vaRenderPicture`, `vaEndPicture`), `va_vpp.h`
//! (`VAProcPipelineParameterBuffer`) and `va_drmcommon.h`
//! (`VADRMPRIMESurfaceDescriptor`, `VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2`).
//!
//! **DMA-BUF (BK8b, B-15):** the PipeWire stream fills a `DmaBufFrame`
//! from the buffer (`spa_data` fd, chunk offsets and strides, the
//! negotiated DRM format and modifier); `VaH264Session::import_dmabuf`
//! imports it with `import_dmabuf` on the session's display and converts
//! and scales it with `convert` into the encoder input without a CPU copy.
use std::ffi::{c_int, c_uint, c_void};
use std::io;

use super::vaapi_ffi::{
    VaDisplay, VaDrmPrimeLayer, VaDrmPrimeObject, VaDrmPrimeSurfaceDescriptor, VaGenericUnion,
    VaId, VaProcColorProperties, VaProcPipelineParameterBuffer, VaRectangle, VaSurfaceAttrib,
    VA_ENTRYPOINT_VIDEO_PROC, VA_FOURCC_BGRA, VA_FOURCC_BGRX, VA_GENERIC_VALUE_TYPE_INTEGER,
    VA_GENERIC_VALUE_TYPE_POINTER, VA_INVALID_ID, VA_PROC_COLOR_STANDARD_BT709,
    VA_PROC_COLOR_STANDARD_SRGB, VA_PROC_PIPELINE_PARAMETER_BUFFER, VA_PROFILE_NONE,
    VA_PROGRESSIVE, VA_RT_FORMAT_RGB32, VA_SOURCE_RANGE_FULL, VA_SOURCE_RANGE_REDUCED,
    VA_SURFACE_ATTRIB_EXTERNAL_BUFFER_DESCRIPTOR, VA_SURFACE_ATTRIB_MEMORY_TYPE,
    VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2, VA_SURFACE_ATTRIB_SETTABLE,
};
use super::vaapi_surface::create_surfaces;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;

/// Parameter buffers of one picture, destroyed when dropped.
pub(super) struct Buffers<'a> {
    display: &'a VaDisplay,
    context: VaId,
    ids: Vec<VaId>,
}

impl<'a> Buffers<'a> {
    pub(super) fn new(display: &'a VaDisplay, context: VaId) -> Self {
        Self {
            display,
            context,
            ids: Vec::new(),
        }
    }

    /// A buffer holding `value`, a `repr(C)` parameter struct.
    pub(super) fn add<T: Copy>(&mut self, kind: c_int, value: &T) -> io::Result<VaId> {
        self.add_elements(kind, std::slice::from_ref(value))
    }

    pub(super) fn add_bytes(&mut self, kind: c_int, bytes: &[u8]) -> io::Result<VaId> {
        let len =
            c_uint::try_from(bytes.len()).map_err(|_| invalid("VA parameter buffer too large"))?;
        let mut id = VA_INVALID_ID;
        // SAFETY: valid context; the driver copies `bytes` at creation
        // (vaCreateBuffer with initial data).
        let status = unsafe {
            (self.display.api.create_buffer)(
                self.display.handle,
                self.context,
                kind,
                len,
                1,
                bytes.as_ptr() as *mut c_void,
                &mut id,
            )
        };
        self.display.check(status, "vaCreateBuffer (parameters)")?;
        self.ids.push(id);
        Ok(id)
    }

    /// One buffer of `values.len()` elements of `T` (BK4b: the per-tile
    /// `VASliceParameterBufferAV1` of a tile group, `num_elements` > 1).
    pub(super) fn add_elements<T: Copy>(&mut self, kind: c_int, values: &[T]) -> io::Result<VaId> {
        if values.is_empty() {
            return Err(invalid("VA parameter buffer without elements"));
        }
        let elem_size = c_uint::try_from(std::mem::size_of::<T>())
            .map_err(|_| invalid("VA element too large"))?;
        let count =
            c_uint::try_from(values.len()).map_err(|_| invalid("VA element count too large"))?;
        let mut id = VA_INVALID_ID;
        // SAFETY: valid context; `values` holds `len` plain `repr(C)` structs
        // of `size_of::<T>()` bytes each, which the driver copies at
        // creation (vaCreateBuffer with initial data, va.h).
        let status = unsafe {
            (self.display.api.create_buffer)(
                self.display.handle,
                self.context,
                kind,
                elem_size,
                count,
                values.as_ptr() as *mut c_void,
                &mut id,
            )
        };
        self.display.check(status, "vaCreateBuffer (elements)")?;
        self.ids.push(id);
        Ok(id)
    }

    /// Begins a picture on `target`, renders every buffer in the order
    /// added and ends the picture.
    pub(super) fn submit(&mut self, target: VaId) -> io::Result<()> {
        let api = &self.display.api;
        let handle = self.display.handle;
        // SAFETY: context, target surface and buffers belong to this display.
        let begin = unsafe { (api.begin_picture)(handle, self.context, target) };
        self.display.check(begin, "vaBeginPicture")?;
        // SAFETY: as above; `ids` lives through the call.
        let render = unsafe {
            (api.render_picture)(
                handle,
                self.context,
                self.ids.as_mut_ptr(),
                self.ids.len() as c_int,
            )
        };
        // SAFETY: a begun picture is always ended.
        let end = unsafe { (api.end_picture)(handle, self.context) };
        self.display.check(render, "vaRenderPicture")?;
        self.display.check(end, "vaEndPicture")
    }
}

impl Drop for Buffers<'_> {
    fn drop(&mut self) {
        for id in &self.ids {
            // SAFETY: each buffer was created here and is no longer in use.
            unsafe { (self.display.api.destroy_buffer)(self.display.handle, *id) };
        }
    }
}

/// The video processor's conversion of `source` (RGB, full range sRGB)
/// into `target` (NV12, BT.709 limited range, the matrix every decoder of
/// this program assumes): the whole `from` picture of the source scaled to
/// the `to` picture of the target, both from the top left corner (equal
/// sizes: no scaling; BK8b scales a DMA-BUF to the viewer's size here).
pub(super) fn convert(
    display: &VaDisplay,
    context: VaId,
    source: VaId,
    from: FrameSize,
    target: VaId,
    to: FrameSize,
) -> io::Result<()> {
    let rectangle = |size: FrameSize| -> io::Result<VaRectangle> {
        Ok(VaRectangle {
            x: 0,
            y: 0,
            width: u16::try_from(size.width).map_err(|_| invalid("VA rectangle exceeds u16"))?,
            height: u16::try_from(size.height).map_err(|_| invalid("VA rectangle exceeds u16"))?,
        })
    };
    let (region, output) = (rectangle(from)?, rectangle(to)?);
    // SAFETY: plain C data; all-zero is the "no filter, no reference, no
    // rotation" state of the pipeline buffer (va_vpp.h).
    let mut pipeline: VaProcPipelineParameterBuffer = unsafe { std::mem::zeroed() };
    pipeline.surface = source;
    pipeline.surface_region = &region;
    pipeline.surface_color_standard = VA_PROC_COLOR_STANDARD_SRGB;
    pipeline.output_region = &output;
    pipeline.output_color_standard = VA_PROC_COLOR_STANDARD_BT709;
    pipeline.input_color_properties = VaProcColorProperties {
        color_range: VA_SOURCE_RANGE_FULL,
        ..VaProcColorProperties::default()
    };
    pipeline.output_color_properties = VaProcColorProperties {
        color_range: VA_SOURCE_RANGE_REDUCED,
        ..VaProcColorProperties::default()
    };
    // `region` and `output` stay borrowed by `pipeline` until the picture
    // ended below.
    let mut buffers = Buffers::new(display, context);
    buffers.add(VA_PROC_PIPELINE_PARAMETER_BUFFER, &pipeline)?;
    buffers.submit(target)
}

/// The VA video processor of an encoder: its config, its context writing
/// into the encoder's input surface, and the RGB surface the captured BGRA
/// goes into. Closed explicitly (`close`), as it needs the display.
pub(super) struct Converter {
    pub config: VaId,
    pub context: VaId,
    pub rgb: VaId,
    pub fourcc: u32,
}

impl Converter {
    /// Opens the video processor for pictures of `size` into `target` (an
    /// NV12 surface of `coded` size).
    pub(super) fn open(
        display: &VaDisplay,
        size: FrameSize,
        coded: FrameSize,
        target: VaId,
    ) -> io::Result<Self> {
        let mut config = VA_INVALID_ID;
        // SAFETY: valid display; no attributes.
        let status = unsafe {
            (display.api.create_config)(
                display.handle,
                VA_PROFILE_NONE,
                VA_ENTRYPOINT_VIDEO_PROC,
                std::ptr::null_mut(),
                0,
                &mut config,
            )
        };
        display.check(status, "vaCreateConfig (video processor)")?;
        let mut converter = Self {
            config,
            context: VA_INVALID_ID,
            rgb: VA_INVALID_ID,
            fourcc: 0,
        };
        let mut target = target;
        let mut opened = || -> io::Result<()> {
            (converter.rgb, converter.fourcc) = create_rgb_surface(display, size)?;
            // SAFETY: config and target surface belong to this display.
            let status = unsafe {
                (display.api.create_context)(
                    display.handle,
                    config,
                    coded.width as c_int,
                    coded.height as c_int,
                    VA_PROGRESSIVE,
                    &mut target,
                    1,
                    &mut converter.context,
                )
            };
            display.check(status, "vaCreateContext (video processor)")
        };
        match opened() {
            Ok(()) => Ok(converter),
            Err(error) => {
                converter.close(display);
                Err(error)
            }
        }
    }

    pub(super) fn close(&self, display: &VaDisplay) {
        // SAFETY: every id belongs to this display; invalid ids are skipped.
        unsafe {
            if self.context != VA_INVALID_ID {
                (display.api.destroy_context)(display.handle, self.context);
            }
            if self.rgb != VA_INVALID_ID {
                let mut rgb = self.rgb;
                (display.api.destroy_surfaces)(display.handle, &mut rgb, 1);
            }
            (display.api.destroy_config)(display.handle, self.config);
        }
    }
}

/// The RGB32 source surface of the video processor and its layout.
pub(super) fn create_rgb_surface(display: &VaDisplay, size: FrameSize) -> io::Result<(VaId, u32)> {
    let mut last = invalid("no RGB surface");
    for fourcc in [VA_FOURCC_BGRA, VA_FOURCC_BGRX] {
        match create_surfaces(display, VA_RT_FORMAT_RGB32, size, 1, Some(fourcc)) {
            Ok(surfaces) => return Ok((surfaces[0], fourcc)),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// One DMA-BUF picture from the compositor (filled by the PipeWire stream,
/// BK8b). `fd` is borrowed: the holder of the frame keeps its own duplicate
/// open until the import surface is destroyed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::screen::os) struct DmaBufFrame {
    pub fd: c_int,
    /// DRM FOURCC of the picture (for example `XR24`).
    pub drm_format: u32,
    pub modifier: u64,
    pub size: FrameSize,
    /// Offset and pitch of each plane of the one buffer object.
    pub planes: Vec<(u32, u32)>,
    pub object_size: u32,
}

/// The import description of `frame` (one object, one layer).
pub(super) fn prime_descriptor(
    frame: &DmaBufFrame,
    fourcc: u32,
) -> io::Result<VaDrmPrimeSurfaceDescriptor> {
    if frame.planes.is_empty() || frame.planes.len() > 4 {
        return Err(invalid("DMA-BUF frame needs one to four planes"));
    }
    let mut layer = VaDrmPrimeLayer {
        drm_format: frame.drm_format,
        num_planes: frame.planes.len() as u32,
        ..VaDrmPrimeLayer::default()
    };
    for (index, (offset, pitch)) in frame.planes.iter().enumerate() {
        layer.offset[index] = *offset;
        layer.pitch[index] = *pitch;
    }
    let mut descriptor = VaDrmPrimeSurfaceDescriptor {
        fourcc,
        width: frame.size.width,
        height: frame.size.height,
        num_objects: 1,
        num_layers: 1,
        ..VaDrmPrimeSurfaceDescriptor::default()
    };
    descriptor.objects[0] = VaDrmPrimeObject {
        fd: frame.fd,
        size: frame.object_size,
        drm_format_modifier: frame.modifier,
    };
    descriptor.layers[0] = layer;
    Ok(descriptor)
}

/// A VA surface over the DMA-BUF of `frame` (no copy); `fourcc` is the VA
/// FOURCC of its layout (`VA_FOURCC_BGRX` for `XR24`). The caller destroys
/// the surface with `vaapi_surface::destroy_surfaces` once the video
/// processor read it. libva 2.20.0 `va_drmcommon.h`: the driver does not
/// close the descriptor, and closing it does not end the surface.
pub(super) fn import_dmabuf(
    display: &VaDisplay,
    frame: &DmaBufFrame,
    fourcc: u32,
) -> io::Result<VaId> {
    let mut descriptor = prime_descriptor(frame, fourcc)?;
    let mut attribs = [
        VaSurfaceAttrib {
            kind: VA_SURFACE_ATTRIB_MEMORY_TYPE,
            flags: VA_SURFACE_ATTRIB_SETTABLE,
            value_type: VA_GENERIC_VALUE_TYPE_INTEGER,
            value: VaGenericUnion {
                i: VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2 as i32,
            },
        },
        VaSurfaceAttrib {
            kind: VA_SURFACE_ATTRIB_EXTERNAL_BUFFER_DESCRIPTOR,
            flags: VA_SURFACE_ATTRIB_SETTABLE,
            value_type: VA_GENERIC_VALUE_TYPE_POINTER,
            value: VaGenericUnion {
                p: (&mut descriptor as *mut VaDrmPrimeSurfaceDescriptor).cast(),
            },
        },
    ];
    let mut surface = VA_INVALID_ID;
    // SAFETY: `descriptor` and `attribs` live through the call; the driver
    // imports the buffer object during vaCreateSurfaces.
    let status = unsafe {
        (display.api.create_surfaces)(
            display.handle,
            VA_RT_FORMAT_RGB32,
            frame.size.width,
            frame.size.height,
            &mut surface,
            1,
            attribs.as_mut_ptr(),
            attribs.len() as c_uint,
        )
    };
    display.check(status, "vaCreateSurfaces (DMA-BUF)")?;
    Ok(surface)
}
