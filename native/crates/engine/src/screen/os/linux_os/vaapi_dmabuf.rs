//! Which DMA-BUF layouts the VA-API encoder of this host imports (plan
//! 2026-11-14, package BK8b, B-15), and the one read-back of an imported
//! picture a still image may make.
//!
//! libva has no query for importable modifiers: `VASurfaceAttribDRMFormat
//! Modifiers` is write-only and "can only be used when allocating a new
//! buffer, it's invalid to use this attribute when importing" (libva 2.20.0
//! `va.h`). The probe therefore lets the driver allocate an RGB32 surface
//! (once with its own choice, once asked for `DRM_FORMAT_MOD_LINEAR`),
//! exports it with `vaExportSurfaceHandle` (`VA_SURFACE_ATTRIB_MEM_TYPE_DRM
//! _PRIME_2`, read only, composed layers) and imports the exported buffer
//! back with the import of the encoder (`vaapi_vpp::import_dmabuf`). A
//! modifier is offered only when that round trip worked. Sunshine instead
//! asks EGL (`eglQueryDmaBufModifiersEXT`, R01 `pipewire.cpp`), because it
//! imports into EGL; this program imports into VA, so it asks VA.
//!
//! Sources (libva 2.20.0, read on 2026-11-14): `va.h` (`VASurfaceAttribType`
//! counted: None 0, PixelFormat 1, MinWidth 2, MaxWidth 3, MinHeight 4,
//! MaxHeight 5, MemoryType 6, ExternalBufferDescriptor 7, UsageHint 8,
//! DRMFormatModifiers 9; `VA_EXPORT_SURFACE_READ_ONLY` 0x0001,
//! `VA_EXPORT_SURFACE_COMPOSED_LAYERS` 0x0008; `vaExportSurfaceHandle(dpy,
//! surface, uint32_t mem_type, uint32_t flags, void *descriptor)`: the
//! exported handles are owned by the caller), `va_drmcommon.h`
//! (`VADRMFormatModifierList` = `uint32_t num_modifiers; uint64_t
//! *modifiers;`, 16 bytes on LP64).
use std::ffi::{c_int, c_uint, c_void};
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libloading::Library;

use super::vaapi_ffi::{
    VaDisplay, VaDrmPrimeSurfaceDescriptor, VaGenericUnion, VaHandle, VaId, VaStatus,
    VaSurfaceAttrib, VA_FOURCC_BGRA, VA_FOURCC_BGRX, VA_GENERIC_VALUE_TYPE_INTEGER,
    VA_GENERIC_VALUE_TYPE_POINTER, VA_INVALID_ID, VA_RT_FORMAT_RGB32,
    VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2, VA_SURFACE_ATTRIB_PIXEL_FORMAT,
    VA_SURFACE_ATTRIB_SETTABLE,
};
use super::vaapi_surface::{destroy_surfaces, read_rgb32};
use super::vaapi_vpp::{import_dmabuf, DmaBufFrame};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;

const VA_SURFACE_ATTRIB_DRM_FORMAT_MODIFIERS: c_int = 9;
const VA_EXPORT_SURFACE_READ_ONLY: u32 = 0x0001;
const VA_EXPORT_SURFACE_COMPOSED_LAYERS: u32 = 0x0008;
/// `drm_fourcc.h` (Linux 6.8): 'XR24', 'AR24', the linear modifier.
pub(in crate::screen::os) const DRM_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub(in crate::screen::os) const DRM_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
const DRM_MOD_LINEAR: u64 = 0;
/// Size of the probe surfaces (any size the drivers allocate).
const PROBE_SIZE: FrameSize = FrameSize::new(256, 256);

/// `VADRMFormatModifierList`.
#[repr(C)]
#[allow(dead_code)] // Read by the driver, not by this program.
struct VaDrmFormatModifierList {
    num_modifiers: u32,
    modifiers: *mut u64,
}

type ExportFn = unsafe extern "C" fn(VaHandle, VaId, u32, u32, *mut c_void) -> VaStatus;

struct Export {
    _library: Library,
    export: ExportFn,
}

/// `vaExportSurfaceHandle` (libva ≥ 2.1), loaded once; `None` without it.
fn export_fn() -> Option<ExportFn> {
    static EXPORT: OnceLock<Option<Export>> = OnceLock::new();
    EXPORT
        .get_or_init(|| {
            // SAFETY: libva is already loaded by `vaapi_ffi::load`; this
            // takes a second reference and runs no new initializer.
            let library = unsafe { Library::new(super::vaapi_ffi::LIBVA) }.ok()?;
            // SAFETY: the signature is the one of va.h 2.20.0 above.
            let export = unsafe { library.get::<ExportFn>(b"vaExportSurfaceHandle\0") }
                .ok()
                .map(|symbol| *symbol)?;
            Some(Export {
                _library: library,
                export,
            })
        })
        .as_ref()
        .map(|found| found.export)
}

/// The VA FOURCC of the memory layout of a DRM format (`XR24` = bytes B, G,
/// R, x = `VA_FOURCC_BGRX`; `AR24` = `VA_FOURCC_BGRA`).
pub(in crate::screen::os) fn va_fourcc_for(drm_format: u32) -> Option<u32> {
    match drm_format {
        DRM_XRGB8888 => Some(VA_FOURCC_BGRX),
        DRM_ARGB8888 => Some(VA_FOURCC_BGRA),
        _ => None,
    }
}

/// An RGB32 surface of `fourcc`, with the linear modifier when `linear`.
fn allocate(display: &VaDisplay, fourcc: u32, linear: bool) -> io::Result<VaId> {
    let mut modifiers = [DRM_MOD_LINEAR];
    let mut list = VaDrmFormatModifierList {
        num_modifiers: 1,
        modifiers: modifiers.as_mut_ptr(),
    };
    let mut attribs = vec![VaSurfaceAttrib {
        kind: VA_SURFACE_ATTRIB_PIXEL_FORMAT,
        flags: VA_SURFACE_ATTRIB_SETTABLE,
        value_type: VA_GENERIC_VALUE_TYPE_INTEGER,
        value: VaGenericUnion { i: fourcc as i32 },
    }];
    if linear {
        attribs.push(VaSurfaceAttrib {
            kind: VA_SURFACE_ATTRIB_DRM_FORMAT_MODIFIERS,
            flags: VA_SURFACE_ATTRIB_SETTABLE,
            value_type: VA_GENERIC_VALUE_TYPE_POINTER,
            value: VaGenericUnion {
                p: (&mut list as *mut VaDrmFormatModifierList).cast(),
            },
        });
    }
    let mut surface = VA_INVALID_ID;
    // SAFETY: `attribs`, `list` and `modifiers` live through the call.
    let status = unsafe {
        (display.api.create_surfaces)(
            display.handle,
            VA_RT_FORMAT_RGB32,
            PROBE_SIZE.width,
            PROBE_SIZE.height,
            &mut surface,
            1,
            attribs.as_mut_ptr(),
            attribs.len() as c_uint,
        )
    };
    display.check(status, "vaCreateSurfaces (modifier probe)")?;
    Ok(surface)
}

/// Exports `surface` and imports it back; the modifier when both worked.
fn round_trip(display: &VaDisplay, surface: VaId, fourcc: u32, drm: u32) -> io::Result<u64> {
    let export = export_fn().ok_or_else(|| invalid("libva lacks vaExportSurfaceHandle"))?;
    let mut descriptor = VaDrmPrimeSurfaceDescriptor::default();
    for object in &mut descriptor.objects {
        object.fd = -1;
    }
    // SAFETY: valid surface; the driver fills `descriptor` (va.h).
    let status = unsafe {
        export(
            display.handle,
            surface,
            VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2,
            VA_EXPORT_SURFACE_READ_ONLY | VA_EXPORT_SURFACE_COMPOSED_LAYERS,
            (&mut descriptor as *mut VaDrmPrimeSurfaceDescriptor).cast(),
        )
    };
    display.check(status, "vaExportSurfaceHandle")?;
    let objects = (descriptor.num_objects as usize).min(4);
    let mut seen = Vec::new();
    for object in &descriptor.objects[..objects] {
        if object.fd >= 0 && !seen.contains(&object.fd) {
            seen.push(object.fd);
        }
    }
    // SAFETY: every distinct exported descriptor is owned by the caller
    // (va.h) and closed exactly once when `_owned` drops.
    let _owned: Vec<OwnedFd> = seen
        .into_iter()
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
        .collect();
    let layer = descriptor.layers[0];
    if objects != 1
        || descriptor.objects[0].fd < 0
        || descriptor.num_layers != 1
        || layer.num_planes != 1
    {
        return Err(invalid("exported surface is not one object with one plane"));
    }
    let frame = DmaBufFrame {
        fd: descriptor.objects[0].fd,
        drm_format: drm,
        modifier: descriptor.objects[0].drm_format_modifier,
        size: PROBE_SIZE,
        planes: vec![(layer.offset[0], layer.pitch[0])],
        object_size: descriptor.objects[0].size,
    };
    let mut imported = [import_dmabuf(display, &frame, fourcc)?];
    destroy_surfaces(display, &mut imported);
    Ok(frame.modifier)
}

/// The modifiers of `drm` this display imports, preferred (the driver's
/// own choice) first.
pub(super) fn importable_modifiers(display: &VaDisplay, drm: u32) -> Vec<u64> {
    let Some(fourcc) = va_fourcc_for(drm) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for linear in [false, true] {
        let Ok(surface) = allocate(display, fourcc, linear) else {
            continue;
        };
        let modifier = round_trip(display, surface, fourcc, drm);
        destroy_surfaces(display, &mut [surface]);
        if let Ok(modifier) = modifier {
            if !found.contains(&modifier) {
                found.push(modifier);
            }
        }
    }
    found
}

/// What the VA encoder on `node` imports: per DRM format its modifiers.
pub(super) fn probe_node(node: &Path) -> Vec<(u32, Vec<u64>)> {
    let Ok(api) = super::vaapi_ffi::load() else {
        return Vec::new();
    };
    let Ok(display) = VaDisplay::open(api, node) else {
        return Vec::new();
    };
    [DRM_XRGB8888, DRM_ARGB8888]
        .into_iter()
        .map(|drm| (drm, importable_modifiers(&display, drm)))
        .filter(|(_, modifiers)| !modifiers.is_empty())
        .collect()
}

/// The device a DMA-BUF capture is imported on and what it imports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::screen::os) struct ImportTarget {
    pub node: Option<PathBuf>,
    pub formats: Vec<(u32, Vec<u64>)>,
}

/// Reads an imported picture back into rows of BGRA (a still image, BK8b
/// result 5): import on `node`, `vaGetImage`, rows to `row`.
pub(in crate::screen::os) fn read_back(
    node: &Path,
    frame: &DmaBufFrame,
    row: &mut dyn FnMut(u32, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let fourcc = va_fourcc_for(frame.drm_format)
        .ok_or_else(|| invalid("DMA-BUF format without a VA layout"))?;
    let api = super::vaapi_ffi::load().map_err(io::Error::other)?;
    let display = VaDisplay::open(api, node)?;
    let mut surface = [import_dmabuf(&display, frame, fourcc)?];
    let result = read_rgb32(&display, surface[0], fourcc, frame.size, row);
    destroy_surfaces(&display, &mut surface);
    result
}
