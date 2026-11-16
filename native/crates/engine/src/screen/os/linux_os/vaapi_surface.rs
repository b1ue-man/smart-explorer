//! VA surfaces of the Linux hardware codecs (BK4): creating surfaces,
//! writing a captured picture into one (BGRA into an RGB surface for the
//! video processor, or NV12 from the CPU conversion) and reading a decoded
//! NV12 surface back (B-38), and reading an imported DMA-BUF picture back
//! for a still image (BK8b, `read_rgb32`). The video processor and the
//! DMA-BUF import are in `vaapi_vpp.rs`, the modifier probe in
//! `vaapi_dmabuf.rs`. Sources: libva 2.20.0 `va.h`
//! (`vaCreateSurfaces` 0.33 form, `vaDeriveImage`, `vaCreateImage`,
//! `vaPutImage`, `vaGetImage`, `vaMapBuffer`).
use std::ffi::{c_int, c_uint, c_void};
use std::io;

use super::vaapi_ffi::{
    VaDisplay, VaGenericUnion, VaId, VaImage, VaImageFormat, VaSurfaceAttrib, VA_FOURCC_NV12,
    VA_GENERIC_VALUE_TYPE_INTEGER, VA_INVALID_ID, VA_RT_FORMAT_YUV420,
    VA_SURFACE_ATTRIB_PIXEL_FORMAT, VA_SURFACE_ATTRIB_SETTABLE,
};
use crate::screen::core::pixel_convert::{BgraView, FrameSize, I420Image};
use crate::screen::core::wire::invalid;

/// NV12 surfaces for the encoder input, reconstructed or decoded pictures.
pub(super) fn create_nv12_surfaces(
    display: &VaDisplay,
    size: FrameSize,
    count: usize,
) -> io::Result<Vec<VaId>> {
    create_surfaces(
        display,
        VA_RT_FORMAT_YUV420,
        size,
        count,
        Some(VA_FOURCC_NV12),
    )
}

/// Creates `count` surfaces of `format` (`VA_RT_FORMAT_*`) and size,
/// optionally with one pixel format attribute (a FOURCC).
pub(super) fn create_surfaces(
    display: &VaDisplay,
    format: u32,
    size: FrameSize,
    count: usize,
    fourcc: Option<u32>,
) -> io::Result<Vec<VaId>> {
    let mut surfaces = vec![VA_INVALID_ID; count];
    let mut attribs: Vec<VaSurfaceAttrib> = fourcc
        .map(|fourcc| VaSurfaceAttrib {
            kind: VA_SURFACE_ATTRIB_PIXEL_FORMAT,
            flags: VA_SURFACE_ATTRIB_SETTABLE,
            value_type: VA_GENERIC_VALUE_TYPE_INTEGER,
            value: VaGenericUnion { i: fourcc as i32 },
        })
        .into_iter()
        .collect();
    // SAFETY: `surfaces` has room for `count` ids; `attribs` lives through
    // the call (vaCreateSurfaces, VA-API 0.33 form).
    let status = unsafe {
        (display.api.create_surfaces)(
            display.handle,
            format,
            size.width,
            size.height,
            surfaces.as_mut_ptr(),
            count as c_uint,
            if attribs.is_empty() {
                std::ptr::null_mut()
            } else {
                attribs.as_mut_ptr()
            },
            attribs.len() as c_uint,
        )
    };
    display.check(status, "vaCreateSurfaces")?;
    Ok(surfaces)
}

pub(super) fn destroy_surfaces(display: &VaDisplay, surfaces: &mut [VaId]) {
    if surfaces.is_empty() {
        return;
    }
    // SAFETY: the surfaces were created on this display and are unused.
    unsafe {
        (display.api.destroy_surfaces)(
            display.handle,
            surfaces.as_mut_ptr(),
            surfaces.len() as c_int,
        )
    };
}

/// A mapped image of a surface: derived (the surface's own memory) or a
/// separate image that `put`/`get` copies.
struct Mapped<'a> {
    display: &'a VaDisplay,
    image: VaImage,
    data: *mut u8,
}

impl Mapped<'_> {
    fn plane(&self, index: usize) -> io::Result<(*mut u8, usize)> {
        if index >= self.image.offsets.len() || index as u32 >= self.image.num_planes {
            return Err(invalid("VA image lacks a plane"));
        }
        let offset = self.image.offsets[index] as usize;
        if offset >= self.image.data_size as usize {
            return Err(invalid("VA image plane outside its buffer"));
        }
        // SAFETY: the offset lies within the mapped `data_size` bytes.
        Ok((
            unsafe { self.data.add(offset) },
            self.image.pitches[index] as usize,
        ))
    }

    /// Row `row` of plane `index`, `width` bytes, checked against the size.
    fn row(&self, index: usize, row: usize, width: usize) -> io::Result<*mut u8> {
        let (start, pitch) = self.plane(index)?;
        let begin = (self.image.offsets[index] as usize)
            .checked_add(
                row.checked_mul(pitch)
                    .ok_or_else(|| invalid("VA row overflow"))?,
            )
            .and_then(|at| at.checked_add(width))
            .ok_or_else(|| invalid("VA row overflow"))?;
        if pitch < width || begin > self.image.data_size as usize {
            return Err(invalid("VA image row outside its buffer"));
        }
        // SAFETY: `row * pitch + width` lies within the mapped buffer.
        Ok(unsafe { start.add(row * pitch) })
    }
}

impl Drop for Mapped<'_> {
    fn drop(&mut self) {
        // SAFETY: the buffer belongs to this display and is unmapped once.
        unsafe { (self.display.api.unmap_buffer)(self.display.handle, self.image.buf) };
    }
}

/// Destroys an image when dropped (after its mapping, which is declared
/// later and therefore dropped first).
struct ImageGuard<'a> {
    display: &'a VaDisplay,
    id: VaId,
}

impl Drop for ImageGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: the image belongs to this display and is destroyed once.
        unsafe { (self.display.api.destroy_image)(self.display.handle, self.id) };
    }
}

fn map_image(display: &VaDisplay, image: VaImage) -> io::Result<Mapped<'_>> {
    let mut data: *mut c_void = std::ptr::null_mut();
    // SAFETY: the image's buffer belongs to this display.
    let status = unsafe { (display.api.map_buffer)(display.handle, image.buf, &mut data) };
    display.check(status, "vaMapBuffer (image)")?;
    if data.is_null() {
        return Err(invalid("vaMapBuffer gave no image memory"));
    }
    Ok(Mapped {
        display,
        image,
        data: data.cast(),
    })
}

fn derive(display: &VaDisplay, surface: VaId, fourcc: u32, size: FrameSize) -> Option<VaImage> {
    let mut image = VaImage::default();
    // SAFETY: valid surface; `image` is written by the driver.
    let status = unsafe { (display.api.derive_image)(display.handle, surface, &mut image) };
    if status != 0 {
        return None;
    }
    if image.format.fourcc != fourcc
        || u32::from(image.width) < size.width
        || u32::from(image.height) < size.height
    {
        // SAFETY: the derived image is released when its layout is unusable.
        unsafe { (display.api.destroy_image)(display.handle, image.image_id) };
        return None;
    }
    Some(image)
}

fn created_image(display: &VaDisplay, fourcc: u32, size: FrameSize) -> io::Result<VaImage> {
    let (width, height) = (
        c_int::try_from(size.width).map_err(|_| invalid("VA image width overflow"))?,
        c_int::try_from(size.height).map_err(|_| invalid("VA image height overflow"))?,
    );
    let mut format = VaImageFormat {
        fourcc,
        byte_order: 1,
        bits_per_pixel: if fourcc == VA_FOURCC_NV12 { 12 } else { 32 },
        ..VaImageFormat::default()
    };
    let mut image = VaImage::default();
    // SAFETY: format and image slots live through the call.
    let status = unsafe {
        (display.api.create_image)(display.handle, &mut format, width, height, &mut image)
    };
    display.check(status, "vaCreateImage")?;
    if image.format.fourcc != fourcc
        || u32::from(image.width) < size.width
        || u32::from(image.height) < size.height
    {
        // SAFETY: the created image belongs to this display and is unusable.
        unsafe { (display.api.destroy_image)(display.handle, image.image_id) };
        return Err(invalid("VA created image differs from requested layout"));
    }
    Ok(image)
}

/// Writes rows into a surface: through a derived image where the driver
/// gives one, else into a separate image followed by `vaPutImage`.
fn write_surface(
    display: &VaDisplay,
    surface: VaId,
    fourcc: u32,
    size: FrameSize,
    fill: impl FnOnce(&Mapped<'_>) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(image) = derive(display, surface, fourcc, size) {
        let _guard = ImageGuard {
            display,
            id: image.image_id,
        };
        let mapped = map_image(display, image)?;
        return fill(&mapped);
    }
    let image = created_image(display, fourcc, size)?;
    let _guard = ImageGuard {
        display,
        id: image.image_id,
    };
    {
        // Unmapped at the end of this scope, before the put.
        let mapped = map_image(display, image)?;
        fill(&mapped)?;
    }
    // SAFETY: surface and image are of this display and of `size`.
    let status = unsafe {
        (display.api.put_image)(
            display.handle,
            surface,
            image.image_id,
            0,
            0,
            size.width,
            size.height,
            0,
            0,
            size.width,
            size.height,
        )
    };
    display.check(status, "vaPutImage")
}

/// Copies a BGRA picture into an RGB32 surface (BGRA or BGRX layout).
pub(super) fn upload_bgra(
    display: &VaDisplay,
    surface: VaId,
    fourcc: u32,
    view: &BgraView<'_>,
) -> io::Result<()> {
    let size = view.size();
    let width = size.width as usize * 4;
    write_surface(display, surface, fourcc, size, |mapped| {
        for y in 0..size.height {
            let target = mapped.row(0, y as usize, width)?;
            let source = view.row(y);
            // SAFETY: `row` checked `width` writable bytes at `target`.
            unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), target, width) };
        }
        Ok(())
    })
}

/// Writes an I420 picture into an NV12 surface (U and V interleaved).
pub(super) fn upload_i420(display: &VaDisplay, surface: VaId, yuv: &I420Image) -> io::Result<()> {
    let size = yuv.size;
    let (width, height) = (size.width as usize, size.height as usize);
    let chroma = size.chroma();
    let (chroma_width, chroma_rows) = (chroma.width as usize, chroma.height as usize);
    if yuv.y.len() < width * height
        || yuv.u.len() < chroma_width * chroma_rows
        || yuv.v.len() < chroma_width * chroma_rows
    {
        return Err(invalid("I420 planes smaller than their size"));
    }
    write_surface(display, surface, VA_FOURCC_NV12, size, |mapped| {
        for row in 0..height {
            let target = mapped.row(0, row, width)?;
            // SAFETY: `row` checked `width` writable bytes at `target`.
            unsafe { std::ptr::copy_nonoverlapping(yuv.y[row * width..].as_ptr(), target, width) };
        }
        for row in 0..chroma_rows {
            let target = mapped.row(1, row, chroma_width * 2)?;
            // SAFETY: `row` checked `2 * chroma_width` writable bytes.
            let line = unsafe { std::slice::from_raw_parts_mut(target, chroma_width * 2) };
            let (u, v) = (
                &yuv.u[row * chroma_width..][..chroma_width],
                &yuv.v[row * chroma_width..][..chroma_width],
            );
            for (pair, (u, v)) in line.chunks_exact_mut(2).zip(u.iter().zip(v)) {
                pair[0] = *u;
                pair[1] = *v;
            }
        }
        Ok(())
    })
}

/// Reads a decoded NV12 surface into tight planes (`y`: width × height,
/// `uv`: width × chroma rows).
pub(super) fn read_nv12(
    display: &VaDisplay,
    surface: VaId,
    size: FrameSize,
    y: &mut Vec<u8>,
    uv: &mut Vec<u8>,
) -> io::Result<()> {
    let (width, height) = (size.width as usize, size.height as usize);
    let chroma = size.chroma();
    let (rows, pair_width) = (chroma.height as usize, chroma.width as usize * 2);
    let copy = |mapped: &Mapped<'_>, y: &mut Vec<u8>, uv: &mut Vec<u8>| -> io::Result<()> {
        y.clear();
        uv.clear();
        y.reserve(width * height);
        uv.reserve(pair_width * rows);
        for row in 0..height {
            let source = mapped.row(0, row, width)?;
            // SAFETY: `row` checked `width` readable bytes.
            y.extend_from_slice(unsafe { std::slice::from_raw_parts(source, width) });
        }
        for row in 0..rows {
            let source = mapped.row(1, row, pair_width)?;
            // SAFETY: `row` checked `pair_width` readable bytes.
            uv.extend_from_slice(unsafe { std::slice::from_raw_parts(source, pair_width) });
        }
        Ok(())
    };
    if let Some(image) = derive(display, surface, VA_FOURCC_NV12, size) {
        let _guard = ImageGuard {
            display,
            id: image.image_id,
        };
        let mapped = map_image(display, image)?;
        return copy(&mapped, y, uv);
    }
    let image = created_image(display, VA_FOURCC_NV12, size)?;
    let _guard = ImageGuard {
        display,
        id: image.image_id,
    };
    // SAFETY: surface and image are of this display and of `size`.
    let status = unsafe {
        (display.api.get_image)(
            display.handle,
            surface,
            0,
            0,
            size.width,
            size.height,
            image.image_id,
        )
    };
    display.check(status, "vaGetImage")?;
    let mapped = map_image(display, image)?;
    copy(&mapped, y, uv)
}

/// Reads an RGB32 surface (an imported DMA-BUF of BK8b) back as rows of
/// `4 × width` bytes in the byte order of `fourcc` (`VA_FOURCC_BGRX` or
/// `VA_FOURCC_BGRA`): the one read-back a still image may make (plan
/// 2026-11-14 BK8b, result 5). `vaGetImage` into an image of the driver
/// lays a tiled buffer out linearly; a derived image would not (va.h).
pub(super) fn read_rgb32(
    display: &VaDisplay,
    surface: VaId,
    fourcc: u32,
    size: FrameSize,
    row: &mut dyn FnMut(u32, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    display.sync(surface, "vaSyncSurface (read-back)")?;
    let image = created_image(display, fourcc, size)?;
    let _guard = ImageGuard {
        display,
        id: image.image_id,
    };
    // SAFETY: surface and image are of this display and of `size`.
    let status = unsafe {
        (display.api.get_image)(
            display.handle,
            surface,
            0,
            0,
            size.width,
            size.height,
            image.image_id,
        )
    };
    display.check(status, "vaGetImage (RGB32)")?;
    let mapped = map_image(display, image)?;
    let width = size.width as usize * 4;
    for y in 0..size.height {
        let source = mapped.row(0, y as usize, width)?;
        // SAFETY: `row` checked `width` readable bytes at `source`.
        row(y, unsafe { std::slice::from_raw_parts(source, width) })?;
    }
    Ok(())
}
