//! Test-only original BT.709 path from v0.5.184; never used by production.
//! Exact frozen block is checked against the immutable git source on the runner.
use super::pixel_convert::{BgraImage, BgraView, I420Image};
use super::wire::invalid;
use std::io;
pub(crate) const RELEASE_COMMIT: &str = "a814f263096fe69d3746e840b5f3fad3fd7e88a0";
pub(crate) const PIXEL_PATH: &str = "native/src/screen/core/pixel_convert.rs";
pub(crate) const PIXEL_SHA256: &str =
    "322c0c1221f98f3d5993b3f9b84f24cfd6552dcf1b91a5bfc84167b16fb61165";
pub(crate) const CODEC_PATH: &str = "native/src/screen/os/native_ffi/openh264_codec.rs";
pub(crate) const CODEC_SHA256: &str =
    "7f1f1ee52d5da0130a6ad6e21aa4c7655c99f3067fe7b4e504c2c9e514c40ac1";
// BEGIN_FROZEN_PIXELS
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
    let size = src.size();
    size.check_frame()?;
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

// END_FROZEN_PIXELS
