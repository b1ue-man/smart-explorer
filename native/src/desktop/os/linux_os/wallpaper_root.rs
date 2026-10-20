//! The wallpaper of an X11 session without a desktop that keeps one (U34-4,
//! C2.6): the root pixmap named by `_XROOTPMAP_ID` (or `ESETROOT_PMAP_ID`),
//! the property that `feh`, `nitrogen`, `hsetroot` and `xsetroot` set. Its
//! pixels are saved as a PNG file under `$XDG_RUNTIME_DIR/smart-explorer/`
//! (private, 0600) and then read like any image; the mode is `Span`, since
//! the pixmap covers the whole root window (assumption A13).
//!
//! Pixels are taken as the usual 24-bit true colour in 24 or 32 bits per
//! pixel, in the server's byte order; other formats give an error.
use std::path::PathBuf;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder};
use x11rb::rust_connection::RustConnection;

use super::wallpaper::{RootImage, NO_ROOT_PIXMAP};
use super::x11_facts::{intern, values32};

const FILE_NAME: &str = "desktop-root-wallpaper.png";
/// The decoder of the surface refuses larger images as well
/// (`wallpaper_image::MAX_DECODE_SIDE`).
const MAX_SIDE: u16 = 16_384;

/// The id of the root pixmap, when a program set one.
fn pixmap_of(conn: &RustConnection, root: u32) -> Option<u32> {
    ["_XROOTPMAP_ID", "ESETROOT_PMAP_ID"]
        .iter()
        .find_map(|name| {
            let atom = intern(conn, name)?;
            values32(conn, root, atom, AtomEnum::PIXMAP)
                .first()
                .copied()
                .filter(|id| *id != 0)
        })
}

fn connect() -> Result<(RustConnection, u32), String> {
    let (conn, screen) = RustConnection::connect(None).map_err(|error| format!("X11: {error}"))?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .map(|screen| screen.root)
        .ok_or_else(|| "X11: kein Bildschirm".to_string())?;
    Ok((conn, root))
}

/// The id alone (changes when a program sets another pixmap); the poll of
/// the wallpaper compares it instead of rendering the image every time.
pub(super) fn pixmap_id() -> Option<u32> {
    let (conn, root) = connect().ok()?;
    pixmap_of(&conn, root)
}

/// RGB bytes of `data`, `width` x `height` pixels, `bits` per pixel and a
/// row of `stride` bytes.
pub(crate) fn to_rgb(
    data: &[u8],
    (width, height): (usize, usize),
    bits: u8,
    stride: usize,
    little_endian: bool,
) -> Result<Vec<u8>, String> {
    let bytes_per_pixel = match bits {
        24 => 3,
        32 => 4,
        other => {
            return Err(format!(
                "Farbformat mit {other} Bit je Bildpunkt wird nicht gelesen"
            ))
        }
    };
    let mut rgb = Vec::with_capacity(width * height * 3);
    for row in 0..height {
        let line = data
            .get(row * stride..row * stride + width * bytes_per_pixel)
            .ok_or("Bilddaten des Wurzelfensters sind zu kurz")?;
        for pixel in line.chunks_exact(bytes_per_pixel) {
            let (red, green, blue) = match (bytes_per_pixel, little_endian) {
                (3, true) | (4, true) => (pixel[2], pixel[1], pixel[0]),
                (3, false) => (pixel[0], pixel[1], pixel[2]),
                _ => (pixel[1], pixel[2], pixel[3]),
            };
            rgb.extend_from_slice(&[red, green, blue]);
        }
    }
    Ok(rgb)
}

fn encode_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(rgb)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(out)
}

fn target() -> Result<PathBuf, String> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(|dir| PathBuf::from(dir).join("smart-explorer").join(FILE_NAME))
        .ok_or_else(|| "XDG_RUNTIME_DIR fehlt".to_string())
}

/// Saves the root pixmap as a file.
pub(super) fn save() -> Result<RootImage, String> {
    let (conn, root) = connect()?;
    let id = pixmap_of(&conn, root).ok_or_else(|| NO_ROOT_PIXMAP.to_string())?;
    let failed = |error: &dyn std::fmt::Display| format!("X11: {error}");
    let geometry = conn
        .get_geometry(id)
        .map_err(|error| failed(&error))?
        .reply()
        .map_err(|error| failed(&error))?;
    if geometry.width == 0
        || geometry.height == 0
        || geometry.width > MAX_SIDE
        || geometry.height > MAX_SIDE
    {
        return Err(format!(
            "Das Wurzelbild ({}×{}) hat eine nicht lesbare Größe",
            geometry.width, geometry.height
        ));
    }
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            id,
            0,
            0,
            geometry.width,
            geometry.height,
            u32::MAX,
        )
        .map_err(|error| failed(&error))?
        .reply()
        .map_err(|error| failed(&error))?;
    let setup = conn.setup();
    let format = setup
        .pixmap_formats
        .iter()
        .find(|format| format.depth == image.depth)
        .ok_or("Der Server nennt kein Format für das Wurzelbild")?;
    let pad = usize::from(format.scanline_pad.max(8));
    let bits = usize::from(format.bits_per_pixel);
    let stride = (usize::from(geometry.width) * bits).div_ceil(pad) * pad / 8;
    let rgb = to_rgb(
        &image.data,
        (usize::from(geometry.width), usize::from(geometry.height)),
        format.bits_per_pixel,
        stride,
        setup.image_byte_order == ImageOrder::LSB_FIRST,
    )?;
    let png = encode_png(&rgb, u32::from(geometry.width), u32::from(geometry.height))?;
    let path = target()?;
    crate::support_dirs::write_private_atomic(&path, &png)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(RootImage { path })
}

#[cfg(test)]
#[path = "x11_pixmap_tests.rs"]
mod tests;
