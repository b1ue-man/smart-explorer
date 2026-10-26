//! Separable image reconstruction in the encoded RGB/YUV signal domain. Reduction integrates every
//! overlapping source texel (box area); enlargement uses Catmull-Rom cubic
//! convolution (a=-0.5, four taps). Native pixels bypass reconstruction.
use super::screen_image_memory::{overflow, CpuPixels, PixelAllocation};
use crate::screen::core::image_admission::ImageRole;
use eframe::egui::{Color32, ColorImage};
use std::sync::Arc;

use crate::screen::core::pixel_convert::YuvMatrix;

/// The factors of the shader: `r = y + a·cr`, `g = y − b·cb − c·cr`,
/// `b = y + d·cb`, with full-range chroma (BT.601 and BT.709).
pub(crate) fn coefficients(matrix: YuvMatrix) -> [f32; 4] {
    match matrix {
        YuvMatrix::Bt601 => [1.402, 0.344_136, 0.714_136, 1.772],
        YuvMatrix::Bt709 => [1.5748, 0.187_324, 0.468_124, 1.8556],
    }
}

/// The shader's arithmetic on the CPU, for one sample.
#[cfg(test)]
pub(crate) fn yuv_to_rgb(y: u8, u: u8, v: u8, matrix: YuvMatrix) -> [u8; 3] {
    let [a, b, c, d] = coefficients(matrix);
    let luma = (f32::from(y) / 255.0 - 16.0 / 255.0) * (255.0 / 219.0);
    let cb = (f32::from(u) / 255.0 - 128.0 / 255.0) * (255.0 / 224.0);
    let cr = (f32::from(v) / 255.0 - 128.0 / 255.0) * (255.0 / 224.0);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        channel(luma + a * cr),
        channel(luma - b * cb - c * cr),
        channel(luma + d * cb),
    ]
}

type Tap = (usize, f32);
fn tap_count(source: usize, target: usize, pixel: usize) -> Option<usize> {
    if source == 0 || target == 0 {
        return None;
    }
    if source < target {
        return Some(4);
    }
    let scale = source as f64 / target as f64;
    let lo = (pixel as f64 * scale).floor() as usize;
    let hi = (((pixel + 1) as f64 * scale).ceil() as usize).min(source);
    hi.checked_sub(lo)
}
fn taps(
    source: usize,
    target: usize,
    pixel: usize,
    cancel: &impl Fn() -> bool,
) -> Option<Vec<Tap>> {
    let scale = source as f64 / target as f64;
    let mut out = Vec::new();
    out.try_reserve_exact(tap_count(source, target, pixel)?)
        .ok()?;
    if scale >= 1.0 {
        let lo = pixel as f64 * scale;
        let hi = (pixel + 1) as f64 * scale;
        let range = lo.floor() as usize..(hi.ceil() as usize).min(source);
        for i in range {
            if cancel() {
                return None;
            }
            out.push((
                i,
                ((hi.min((i + 1) as f64) - lo.max(i as f64)) / scale) as f32,
            ));
        }
    } else {
        let center = (pixel as f64 + 0.5) * scale - 0.5;
        let first = center.floor() as isize;
        for i in first - 1..=first + 2 {
            if cancel() {
                return None;
            }
            let x = (center - i as f64).abs();
            let weight = if x < 1.0 {
                1.5 * x * x * x - 2.5 * x * x + 1.0
            } else if x < 2.0 {
                -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
            } else {
                0.0
            };
            out.push((i.clamp(0, source as isize - 1) as usize, weight as f32));
        }
    }
    Some(out)
}
pub(super) fn scratch_bytes(
    source: [usize; 2],
    target: [usize; 2],
    cancel: &impl Fn() -> bool,
) -> Option<u64> {
    let [sw, sh] = source;
    let [tw, th] = target;
    if [sw, sh, tw, th].contains(&0) {
        return None;
    }
    let mut horizontal = 0usize;
    for x in 0..tw {
        if cancel() {
            return None;
        }
        horizontal = horizontal.checked_add(tap_count(sw, tw, x)?)?;
    }
    let mut vertical = 0usize;
    for y in 0..th {
        if cancel() {
            return None;
        }
        vertical = vertical.max(tap_count(sh, th, y)?);
    }
    let bytes = horizontal
        .checked_add(vertical)?
        .checked_mul(std::mem::size_of::<Tap>())?
        .checked_add(tw.checked_mul(std::mem::size_of::<Vec<Tap>>())?)?
        .checked_add(
            tw.checked_mul(std::mem::size_of::<[f32; 3]>())?
                .checked_mul(6)?,
        )?;
    u64::try_from(bytes).ok()
}
pub(super) fn scratch_limit() -> std::io::Result<u64> {
    // Every admitted Login side is <=4096. Reduction has <=sw+2tw taps;
    // enlargement has four per target pixel; vertical reduction has <=sh.
    let side = 4096u64;
    side.checked_mul(5)
        .and_then(|n| n.checked_mul(std::mem::size_of::<Tap>() as u64))
        .and_then(|n| n.checked_add(side * std::mem::size_of::<Vec<Tap>>() as u64))
        .and_then(|n| n.checked_add(side * 6 * std::mem::size_of::<[f32; 3]>() as u64))
        .ok_or_else(overflow)
}
pub(super) fn resample_admitted(
    source: &CpuPixels,
    target: [usize; 2],
    cancel: &impl Fn() -> bool,
) -> Option<Arc<CpuPixels>> {
    if cancel() {
        return None;
    }
    let output =
        PixelAllocation::new(source.access().clone(), ImageRole::ScaledOutput, target).ok()?;
    let bytes = scratch_bytes(source.size, target, cancel)?;
    let scratch = source
        .access()
        .claim(ImageRole::FilterScratch, bytes)
        .ok()?;
    let image = resample_with_cancel(source, target, cancel);
    // Every temporary vector is destroyed by the returned call before this hold.
    drop(scratch);
    image.map(|image| output.finish(image))
}
// Use the encoded signal domain on both CPU and GPU. The YUV matrix is
// linear in those signals, so RGB and plane reconstruction share the same
// area/cubic kernel rather than giving gray edges different brightness.
fn signal(v: u8) -> f32 {
    f32::from(v) / 255.0
}
fn encoded(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
/// Checked allocation; callers retain the last valid texture on resource failure.
#[cfg(test)]
pub(crate) fn resample(source: &ColorImage, target: [usize; 2]) -> Option<ColorImage> {
    resample_with_cancel(source, target, &|| false)
}
pub(crate) fn resample_with_cancel(
    source: &ColorImage,
    target: [usize; 2],
    cancel: &impl Fn() -> bool,
) -> Option<ColorImage> {
    let [sw, sh] = source.size;
    let [tw, th] = target;
    if [sw, sh, tw, th].contains(&0) || sw.checked_mul(sh)? != source.pixels.len() {
        return None;
    }
    let mut xt = Vec::new();
    xt.try_reserve_exact(tw).ok()?;
    for x in 0..tw {
        if cancel() {
            return None;
        }
        xt.push(taps(sw, tw, x, cancel)?);
    }
    let horizontal = |y: usize| -> Option<Vec<[f32; 3]>> {
        if cancel() {
            return None;
        }
        let mut row = Vec::new();
        row.try_reserve_exact(tw).ok()?;
        for weights in &xt {
            let mut rgb = [0.0; 3];
            for &(x, weight) in weights {
                let p = source.pixels[y * sw + x];
                for (sum, value) in rgb.iter_mut().zip([p.r(), p.g(), p.b()]) {
                    *sum += signal(value) * weight;
                }
            }
            row.push(rgb);
        }
        Some(row)
    };
    let mut cache: [Option<(usize, Vec<[f32; 3]>)>; 4] = std::array::from_fn(|_| None);
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(tw.checked_mul(th)?).ok()?;
    for y in 0..th {
        if cancel() {
            return None;
        }
        let weights = taps(sh, th, y, cancel)?;
        for entry in &mut cache {
            if entry
                .as_ref()
                .is_some_and(|(sy, _)| !weights.iter().any(|(needed, _)| needed == sy))
            {
                *entry = None;
            }
        }
        let mut row = Vec::new();
        row.try_reserve_exact(tw).ok()?;
        row.resize(tw, [0.0; 3]);
        for (sy, weight) in weights {
            if th > sh {
                let index = if let Some(index) = cache
                    .iter()
                    .position(|entry| entry.as_ref().is_some_and(|(key, _)| *key == sy))
                {
                    index
                } else {
                    let index = cache.iter().position(Option::is_none)?;
                    cache[index] = Some((sy, horizontal(sy)?));
                    index
                };
                for (out, p) in row.iter_mut().zip(&cache[index].as_ref()?.1) {
                    for (sum, value) in out.iter_mut().zip(p) {
                        *sum += value * weight;
                    }
                }
            } else {
                for (out, p) in row.iter_mut().zip(horizontal(sy)?) {
                    for (sum, value) in out.iter_mut().zip(p) {
                        *sum += value * weight;
                    }
                }
            }
        }
        pixels.extend(
            row.into_iter()
                .map(|rgb| Color32::from_rgb(encoded(rgb[0]), encoded(rgb[1]), encoded(rgb[2]))),
        );
    }
    Some(ColorImage {
        size: target,
        pixels,
    })
}
