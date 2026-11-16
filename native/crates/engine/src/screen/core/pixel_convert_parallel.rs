//! BGRA → YUV 4:2:0 on all cores (plan 2026-11-14 „Bildkette“ B-19), and
//! NV12 → BGRA for the readback of a picture that lay on the GPU.
//!
//! The forward conversion works on pairs of rows in parallel (rayon) with
//! the integer formulas of the reference `bgra_to_i420_with_limits`: the
//! same coefficients, the same rounding, the 2 × 2 chroma average with the
//! repeated edge pixel. Only the order of the work differs, so the result is
//! bit-identical to the reference, for I420 (OpenH264) and for NV12 (Media
//! Foundation, written straight into the encoder's buffer or a mapped
//! staging texture). One function serves both layouts.
use rayon::prelude::*;

use super::{
    clamp_u8, yuv_to_bgra, BgraView, Coefficients, FrameLimits, FrameSize, I420Image, Nv12View,
    YuvMatrix,
};
use crate::screen::core::wire::invalid;
use std::io;

fn luma_row(k: &Coefficients, row: &[u8], out: &mut [u8]) {
    for (pixel, luma) in row.chunks_exact(4).zip(out.iter_mut()) {
        let (b, g, r) = (
            i32::from(pixel[0]),
            i32::from(pixel[1]),
            i32::from(pixel[2]),
        );
        *luma = clamp_u8(((k.y[0] * r + k.y[1] * g + k.y[2] * b + 128) >> 8) + 16);
    }
}

/// Cb and Cr of the chroma sample `cx` of the row pair `top`/`bottom`.
fn chroma(k: &Coefficients, top: &[u8], bottom: &[u8], cx: usize, last_x: usize) -> (u8, u8) {
    let (x0, x1) = (cx * 2, (cx * 2 + 1).min(last_x));
    let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
    for (line, x) in [(top, x0), (top, x1), (bottom, x0), (bottom, x1)] {
        b += i32::from(line[x * 4]);
        g += i32::from(line[x * 4 + 1]);
        r += i32::from(line[x * 4 + 2]);
    }
    (
        clamp_u8(((k.u[0] * r + k.u[1] * g + k.u[2] * b + 512) >> 10) + 128),
        clamp_u8(((k.v[0] * r + k.v[1] * g + k.v[2] * b + 512) >> 10) + 128),
    )
}

/// Where the chroma of one row pair goes.
enum ChromaRow<'a> {
    /// NV12: Cb and Cr interleaved.
    Interleaved(&'a mut [u8]),
    /// I420: Cb and Cr in planes of their own.
    Planar(&'a mut [u8], &'a mut [u8]),
}

/// One pair of rows: luma of rows `2 cy` and `2 cy + 1` (the second only
/// when it exists) and their chroma.
fn row_pair(
    k: &Coefficients,
    src: &BgraView,
    cy: usize,
    luma: &mut [u8],
    pitch: usize,
    out: ChromaRow<'_>,
) {
    let size = src.size();
    let (width, cw) = (size.width as usize, size.chroma().width as usize);
    let (last_x, last_y) = (width - 1, size.height - 1);
    let first = cy as u32 * 2;
    let top = src.row(first);
    let bottom = src.row((first + 1).min(last_y));
    luma_row(k, top, &mut luma[..width]);
    if first < last_y {
        luma_row(k, bottom, &mut luma[pitch..pitch + width]);
    }
    match out {
        ChromaRow::Interleaved(pair) => {
            for (cx, out) in pair[..cw * 2].chunks_exact_mut(2).enumerate() {
                (out[0], out[1]) = chroma(k, top, bottom, cx, last_x);
            }
        }
        ChromaRow::Planar(u, v) => {
            for (cx, (u, v)) in u[..cw].iter_mut().zip(v[..cw].iter_mut()).enumerate() {
                (*u, *v) = chroma(k, top, bottom, cx, last_x);
            }
        }
    }
}

/// BGRA → NV12 with luma rows `pitch` apart in `y` and chroma rows `pitch`
/// apart in `uv`, on all cores; bit-identical to the reference.
pub fn bgra_to_nv12_parallel(
    src: &BgraView,
    matrix: YuvMatrix,
    y: &mut [u8],
    uv: &mut [u8],
    pitch: usize,
) -> io::Result<()> {
    let _convert = super::super::convert_clock::Timed::start();
    let size = src.size();
    let (width, height) = (size.width as usize, size.height as usize);
    let (cw, ch) = (size.chroma().width as usize, size.chroma().height as usize);
    let span =
        |rows: usize, last: usize| pitch.checked_mul(rows.checked_sub(1)?)?.checked_add(last);
    if width == 0
        || height == 0
        || pitch < cw * 2
        || pitch < width
        || span(height, width).is_none_or(|n| y.len() < n)
        || span(ch, cw * 2).is_none_or(|n| uv.len() < n)
    {
        return Err(invalid("NV12 planes do not hold the picture"));
    }
    let k = matrix.coefficients();
    y.par_chunks_mut(pitch * 2)
        .zip(uv.par_chunks_mut(pitch))
        .take(ch)
        .enumerate()
        .for_each(|(cy, (luma, pair))| {
            row_pair(&k, src, cy, luma, pitch, ChromaRow::Interleaved(pair));
        });
    Ok(())
}

/// BGRA → I420 in three tight planes, on all cores; the planes are
/// allocated as the reference allocates them, and the result is
/// bit-identical to `bgra_to_i420_with_limits`.
pub fn bgra_to_i420_parallel(
    src: &BgraView,
    matrix: YuvMatrix,
    limits: FrameLimits,
) -> io::Result<I420Image> {
    let _convert = super::super::convert_clock::Timed::start();
    let size = src.size();
    limits.check(size)?;
    let chroma = size.chroma();
    let (width, cw) = (size.width as usize, chroma.width as usize);
    if width == 0 || size.height == 0 {
        return Err(invalid("I420 needs a picture"));
    }
    let mut y_plane = vec![0u8; size.pixels() as usize];
    let mut u_plane = vec![0u8; chroma.pixels() as usize];
    let mut v_plane = vec![0u8; chroma.pixels() as usize];
    let k = matrix.coefficients();
    y_plane
        .par_chunks_mut(width * 2)
        .zip(u_plane.par_chunks_mut(cw))
        .zip(v_plane.par_chunks_mut(cw))
        .enumerate()
        .for_each(|(cy, ((luma, u), v))| {
            row_pair(&k, src, cy, luma, width, ChromaRow::Planar(u, v));
        });
    Ok(I420Image {
        size,
        y: y_plane,
        u: u_plane,
        v: v_plane,
    })
}

/// NV12 → BGRA into the tight rows of `dst`, a picture of `size`: each
/// target pixel takes the nearest source sample (`size` may differ from the
/// source by a scaling or by the one column and row an NV12 surface lacks at
/// odd sides). The readback of a picture that lay on the GPU
/// (`capture_picture::GpuSurface::read_back`).
pub fn nv12_to_bgra_resampled(
    src: &Nv12View,
    matrix: YuvMatrix,
    dst: &mut [u8],
    size: FrameSize,
) -> io::Result<()> {
    if size.width == 0 || size.height == 0 || dst.len() != size.checked_bgra_len()? {
        return Err(invalid("BGRA target does not match its size"));
    }
    let k = matrix.coefficients();
    let (sw, sh) = (u64::from(src.size.width), u64::from(src.size.height));
    let (tw, th) = (u64::from(size.width), u64::from(size.height));
    let width = src.size.width as usize;
    let chroma_row = src.size.chroma().width as usize * 2;
    dst.par_chunks_mut(size.width as usize * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let sy = (y as u64 * sh / th) as usize;
            let lumas = &src.y[sy * src.y_stride..][..width];
            let uvs = &src.uv[(sy / 2) * src.uv_stride..][..chroma_row];
            for (x, out) in row.chunks_exact_mut(4).enumerate() {
                let sx = (x as u64 * sw / tw) as usize;
                let pair = (sx / 2) * 2;
                yuv_to_bgra(&k, lumas[sx], uvs[pair], uvs[pair + 1], out);
            }
        });
    Ok(())
}

#[cfg(test)]
#[path = "bildkette_convert_task_tests.rs"]
mod convert_tests;
