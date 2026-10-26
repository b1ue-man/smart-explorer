//! Frame geometry and CPU pixel conversion (decisions C2 and C4): BGRA frames
//! as capture delivers them, I420 for software encoders, I420 and NV12 back
//! to BGRA as the CPU path of the viewer and the reference of its shader, and
//! an area-averaging downscaler for previews and the resolution ladder.
//! Colour is BT.709 or BT.601 in limited range with 8 bits, in fixed point
//! with 8 fractional bits.
use std::io;

use super::frame_budget::FrameLimits;
use super::wire::invalid;

pub(crate) use super::frame_size_caps::{FrameSize, Rect, MAX_FRAME_PIXELS, MAX_FRAME_SIDE};

/// A borrowed BGRA frame with a row pitch, as capture hands it out. Its size
/// is not limited to `MAX_FRAME_PIXELS`: sources larger than that are scaled
/// down before encoding.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BgraView<'a> {
    size: FrameSize,
    stride: usize,
    data: &'a [u8],
}

impl<'a> BgraView<'a> {
    pub(crate) fn new(size: FrameSize, stride: usize, data: &'a [u8]) -> io::Result<Self> {
        let row = usize::try_from(size.width)
            .ok()
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("BGRA row size overflow"))?;
        let rows = size.height as usize;
        let needed = stride
            .checked_mul(rows.saturating_sub(1))
            .and_then(|bytes| bytes.checked_add(row));
        match needed {
            Some(needed) if size.width > 0 && rows > 0 && stride >= row && data.len() >= needed => {
                Ok(Self { size, stride, data })
            }
            _ => Err(invalid(
                "BGRA frame buffer does not match its size and stride",
            )),
        }
    }

    pub(crate) fn size(&self) -> FrameSize {
        self.size
    }

    /// The `width × 4` bytes of row `y`.
    pub(crate) fn row(&self, y: u32) -> &'a [u8] {
        let start = y as usize * self.stride;
        &self.data[start..start + self.size.width as usize * 4]
    }
}

/// An owned, tight BGRA frame within the frame limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BgraImage {
    size: FrameSize,
    data: Vec<u8>,
    regions: Option<super::frame_regions::FrameRegions>,
}

impl BgraImage {
    /// An opaque black frame.
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        size.check_frame()?;
        Self::new_with_limits(size, super::frame_budget::FrameLimits::LEGACY)
    }

    pub(crate) fn new_with_limits(
        size: FrameSize,
        limits: super::frame_budget::FrameLimits,
    ) -> io::Result<Self> {
        let len = limits.check(size)?;
        let mut data = vec![0u8; len];
        for pixel in data.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
        Ok(Self {
            size,
            data,
            regions: None,
        })
    }

    pub(crate) fn from_vec(size: FrameSize, data: Vec<u8>) -> io::Result<Self> {
        Self::from_vec_with_limits(size, data, FrameLimits::LEGACY)
    }

    pub(crate) fn from_vec_with_limits(
        size: FrameSize,
        data: Vec<u8>,
        limits: FrameLimits,
    ) -> io::Result<Self> {
        if data.len() != limits.check(size)? {
            return Err(invalid("BGRA frame data does not match its size"));
        }
        Ok(Self {
            size,
            data,
            regions: None,
        })
    }

    pub(crate) fn size(&self) -> FrameSize {
        self.size
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn regions(&self) -> Option<&super::frame_regions::FrameRegions> {
        self.regions.as_ref()
    }

    pub(crate) fn set_regions(
        &mut self,
        regions: super::frame_regions::FrameRegions,
    ) -> io::Result<()> {
        regions.validate(self.size)?;
        self.regions = Some(regions);
        Ok(())
    }

    pub(crate) fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub(crate) fn view(&self) -> BgraView<'_> {
        BgraView {
            size: self.size,
            stride: self.size.width as usize * 4,
            data: &self.data,
        }
    }

    pub(crate) fn row_mut(&mut self, y: u32) -> &mut [u8] {
        let row = self.size.width as usize * 4;
        let start = y as usize * row;
        &mut self.data[start..start + row]
    }
}

/// Clockwise quarter turns of a BGRA picture; platform adapters supply the orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BgraRotation {
    Identity,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}
impl BgraRotation {
    pub(crate) fn size(self, size: FrameSize) -> FrameSize {
        match self {
            Self::Clockwise90 | Self::Clockwise270 => FrameSize::new(size.height, size.width),
            _ => size,
        }
    }
}
pub(crate) fn rotate_bgra(
    image: BgraImage,
    rotation: BgraRotation,
    limits: FrameLimits,
) -> io::Result<BgraImage> {
    limits.check(image.size())?;
    if rotation == BgraRotation::Identity {
        return Ok(image);
    }
    let from = image.size();
    let mut target = BgraImage::new_with_limits(rotation.size(from), limits)?;
    for y in 0..from.height {
        for (x, pixel) in image.view().row(y).chunks_exact(4).enumerate() {
            let x = x as u32;
            let (dx, dy) = match rotation {
                BgraRotation::Clockwise90 => (from.height - 1 - y, x),
                BgraRotation::Clockwise180 => (from.width - 1 - x, from.height - 1 - y),
                BgraRotation::Clockwise270 => (y, from.width - 1 - x),
                BgraRotation::Identity => (x, y),
            };
            target.row_mut(dy)[dx as usize * 4..dx as usize * 4 + 4].copy_from_slice(pixel);
        }
    }
    Ok(target)
}

/// A tight 4:2:0 frame with three planes, as software encoders take it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct I420Image {
    pub size: FrameSize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

/// A borrowed NV12 frame (MF decoder output) with its two row pitches.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nv12View<'a> {
    size: FrameSize,
    y: &'a [u8],
    y_stride: usize,
    uv: &'a [u8],
    uv_stride: usize,
}

impl<'a> Nv12View<'a> {
    pub(crate) fn new(
        size: FrameSize,
        y: &'a [u8],
        y_stride: usize,
        uv: &'a [u8],
        uv_stride: usize,
    ) -> io::Result<Self> {
        Self::new_with_limits(size, y, y_stride, uv, uv_stride, FrameLimits::LEGACY)
    }
    pub(crate) fn new_with_limits(
        size: FrameSize,
        y: &'a [u8],
        y_stride: usize,
        uv: &'a [u8],
        uv_stride: usize,
        limits: FrameLimits,
    ) -> io::Result<Self> {
        limits.check(size)?;
        let chroma = size.chroma();
        let fits = |plane: &[u8], stride: usize, row: usize, rows: usize| {
            stride >= row
                && stride
                    .checked_mul(rows.saturating_sub(1))
                    .and_then(|bytes| bytes.checked_add(row))
                    .is_some_and(|needed| plane.len() >= needed)
        };
        if fits(y, y_stride, size.width as usize, size.height as usize)
            && fits(
                uv,
                uv_stride,
                chroma.width as usize * 2,
                chroma.height as usize,
            )
        {
            Ok(Self {
                size,
                y,
                y_stride,
                uv,
                uv_stride,
            })
        } else {
            Err(invalid("NV12 planes do not match their size and stride"))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum YuvMatrix {
    Bt601,
    /// Default: HD content; encoders signal it in the VUI.
    #[default]
    Bt709,
}

/// Fixed-point coefficients × 256: forward RGB → YUV (limited range) and
/// inverse YUV → RGB (`luma` = 255/219).
struct Coefficients {
    y: [i32; 3],
    u: [i32; 3],
    v: [i32; 3],
    luma: i32,
    r_v: i32,
    g_u: i32,
    g_v: i32,
    b_u: i32,
}

impl YuvMatrix {
    fn coefficients(self) -> Coefficients {
        match self {
            Self::Bt601 => Coefficients {
                y: [66, 129, 25],
                u: [-38, -74, 112],
                v: [112, -94, -18],
                luma: 298,
                r_v: 409,
                g_u: 100,
                g_v: 208,
                b_u: 516,
            },
            Self::Bt709 => Coefficients {
                y: [47, 157, 16],
                u: [-26, -86, 112],
                v: [112, -102, -10],
                luma: 298,
                r_v: 459,
                g_u: 55,
                g_v: 136,
                b_u: 541,
            },
        }
    }
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// BGRA → I420. Odd sides are allowed: the last chroma sample repeats the
/// edge pixel. Alpha is ignored.
pub(crate) fn bgra_to_i420(src: &BgraView, matrix: YuvMatrix) -> io::Result<I420Image> {
    bgra_to_i420_with_limits(src, matrix, FrameLimits::LEGACY)
}
pub(crate) fn bgra_to_i420_with_limits(
    src: &BgraView,
    matrix: YuvMatrix,
    limits: FrameLimits,
) -> io::Result<I420Image> {
    let size = src.size();
    limits.check(size)?;
    let k = matrix.coefficients();
    let chroma = size.chroma();
    let mut y_plane = vec![0u8; size.pixels() as usize];
    let mut u_plane = vec![0u8; chroma.pixels() as usize];
    let mut v_plane = vec![0u8; chroma.pixels() as usize];
    for (row, out) in y_plane.chunks_exact_mut(size.width as usize).enumerate() {
        for (pixel, luma) in src.row(row as u32).chunks_exact(4).zip(out.iter_mut()) {
            let (b, g, r) = (
                i32::from(pixel[0]),
                i32::from(pixel[1]),
                i32::from(pixel[2]),
            );
            *luma = clamp_u8(((k.y[0] * r + k.y[1] * g + k.y[2] * b + 128) >> 8) + 16);
        }
    }
    let last_x = size.width as usize - 1;
    let last_y = size.height - 1;
    for (cy, (u_row, v_row)) in u_plane
        .chunks_exact_mut(chroma.width as usize)
        .zip(v_plane.chunks_exact_mut(chroma.width as usize))
        .enumerate()
    {
        let top = src.row(cy as u32 * 2);
        let bottom = src.row((cy as u32 * 2 + 1).min(last_y));
        for (cx, (u, v)) in u_row.iter_mut().zip(v_row.iter_mut()).enumerate() {
            let (x0, x1) = (cx * 2, (cx * 2 + 1).min(last_x));
            let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
            for (line, x) in [(top, x0), (top, x1), (bottom, x0), (bottom, x1)] {
                b += i32::from(line[x * 4]);
                g += i32::from(line[x * 4 + 1]);
                r += i32::from(line[x * 4 + 2]);
            }
            *u = clamp_u8(((k.u[0] * r + k.u[1] * g + k.u[2] * b + 512) >> 10) + 128);
            *v = clamp_u8(((k.v[0] * r + k.v[1] * g + k.v[2] * b + 512) >> 10) + 128);
        }
    }
    Ok(I420Image {
        size,
        y: y_plane,
        u: u_plane,
        v: v_plane,
    })
}

fn yuv_to_bgra(k: &Coefficients, luma: u8, u: u8, v: u8, out: &mut [u8]) {
    let c = k.luma * (i32::from(luma) - 16);
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    out[0] = clamp_u8((c + k.b_u * d + 128) >> 8);
    out[1] = clamp_u8((c - k.g_u * d - k.g_v * e + 128) >> 8);
    out[2] = clamp_u8((c + k.r_v * e + 128) >> 8);
    out[3] = 0xFF;
}

/// I420 → BGRA into `dst` of the same size.
pub(crate) fn i420_to_bgra(
    src: &I420Image,
    matrix: YuvMatrix,
    dst: &mut BgraImage,
) -> io::Result<()> {
    let size = src.size;
    let chroma = size.chroma();
    if dst.size() != size
        || src.y.len() != size.pixels() as usize
        || src.u.len() != chroma.pixels() as usize
        || src.v.len() != chroma.pixels() as usize
    {
        return Err(invalid("I420 planes do not match the target frame"));
    }
    let k = matrix.coefficients();
    let (width, chroma_width) = (size.width as usize, chroma.width as usize);
    for y in 0..size.height {
        let lumas = &src.y[y as usize * width..][..width];
        let us = &src.u[(y / 2) as usize * chroma_width..][..chroma_width];
        let vs = &src.v[(y / 2) as usize * chroma_width..][..chroma_width];
        for (x, (out, luma)) in dst.row_mut(y).chunks_exact_mut(4).zip(lumas).enumerate() {
            yuv_to_bgra(&k, *luma, us[x / 2], vs[x / 2], out);
        }
    }
    Ok(())
}

/// NV12 → BGRA into `dst` of the same size.
pub(crate) fn nv12_to_bgra(
    src: &Nv12View,
    matrix: YuvMatrix,
    dst: &mut BgraImage,
) -> io::Result<()> {
    if dst.size() != src.size {
        return Err(invalid("NV12 frame does not match the target frame"));
    }
    let k = matrix.coefficients();
    let width = src.size.width as usize;
    let chroma_row = src.size.chroma().width as usize * 2;
    for y in 0..src.size.height {
        let lumas = &src.y[y as usize * src.y_stride..][..width];
        let uvs = &src.uv[(y / 2) as usize * src.uv_stride..][..chroma_row];
        for (x, (out, luma)) in dst.row_mut(y).chunks_exact_mut(4).zip(lumas).enumerate() {
            let pair = (x / 2) * 2;
            yuv_to_bgra(&k, *luma, uvs[pair], uvs[pair + 1], out);
        }
    }
    Ok(())
}

/// Area average down to `size`, which must not exceed the source in either
/// side; every source pixel counts once per covered target pixel.
pub(crate) fn downscale_bgra(src: &BgraView, size: FrameSize) -> io::Result<BgraImage> {
    downscale_bgra_with_limits(src, size, FrameLimits::LEGACY)
}
pub(crate) fn downscale_bgra_with_limits(
    src: &BgraView,
    size: FrameSize,
    limits: FrameLimits,
) -> io::Result<BgraImage> {
    let from = src.size();
    if size.width > from.width || size.height > from.height {
        return Err(invalid("downscale target is larger than the source"));
    }
    let mut out = BgraImage::new_with_limits(size, limits)?;
    let span = |index: u32, target: u32, source: u32| {
        let start = (u64::from(index) * u64::from(source) / u64::from(target)) as usize;
        let end = (u64::from(index + 1) * u64::from(source) / u64::from(target)) as usize;
        (start, end.max(start + 1))
    };
    let columns: Vec<(usize, usize)> = (0..size.width)
        .map(|x| span(x, size.width, from.width))
        .collect();
    for y in 0..size.height {
        let (top, bottom) = span(y, size.height, from.height);
        let out_row = out.row_mut(y);
        for (pixel, &(left, right)) in out_row.chunks_exact_mut(4).zip(&columns) {
            let mut sums = [0u64; 4];
            for line in top..bottom {
                for source in src.row(line as u32)[left * 4..right * 4].chunks_exact(4) {
                    for (sum, value) in sums.iter_mut().zip(source) {
                        *sum += u64::from(*value);
                    }
                }
            }
            let count = ((bottom - top) * (right - left)) as u64;
            for (value, sum) in pixel.iter_mut().zip(sums) {
                *value = ((sum + count / 2) / count) as u8;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "pixel_convert_tests.rs"]
mod tests;
