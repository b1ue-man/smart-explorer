//! The mouse pointer in X11 pictures (SCR-M5, U19-6): the server does not
//! draw it into the root window's pixels, so every picture gets the image
//! from XFixes (`GetCursorImage`, ARGB32 with premultiplied alpha) blended
//! onto the CPU copy. Outside the captured area nothing is drawn.
use std::io;

use x11rb::connection::Connection;
use x11rb::protocol::xfixes::ConnectionExt as _;

use crate::screen::core::pixel_convert::FrameSize;

/// The pointer's image and where its top-left corner is on the root window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CursorImage {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// `width * height` pixels as `0xAARRGGBB`, colors premultiplied.
    pub argb: Vec<u32>,
}

/// Announces XFixes 4.0 to the server (required before the first use).
pub(crate) fn enable<C: Connection>(conn: &C) -> io::Result<()> {
    conn.xfixes_query_version(4, 0)
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?;
    Ok(())
}

/// The current pointer image.
pub(crate) fn fetch<C: Connection>(conn: &C) -> io::Result<CursorImage> {
    let reply = conn
        .xfixes_get_cursor_image()
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?;
    Ok(CursorImage {
        x: i32::from(reply.x) - i32::from(reply.xhot),
        y: i32::from(reply.y) - i32::from(reply.yhot),
        width: u32::from(reply.width),
        height: u32::from(reply.height),
        argb: reply.cursor_image,
    })
}

/// Blends `cursor` onto the BGRA picture `target` of `size`, whose top-left
/// corner is at (`origin_x`, `origin_y`) on the root window.
pub(crate) fn blend(
    target: &mut [u8],
    size: FrameSize,
    origin_x: i32,
    origin_y: i32,
    cursor: &CursorImage,
) {
    let (width, height) = (size.width as i64, size.height as i64);
    for row in 0..i64::from(cursor.height) {
        let y = i64::from(cursor.y) + row - i64::from(origin_y);
        if !(0..height).contains(&y) {
            continue;
        }
        for column in 0..i64::from(cursor.width) {
            let x = i64::from(cursor.x) + column - i64::from(origin_x);
            if !(0..width).contains(&x) {
                continue;
            }
            let Some(source) = cursor
                .argb
                .get((row * i64::from(cursor.width) + column) as usize)
            else {
                return;
            };
            let at = ((y * width + x) * 4) as usize;
            let Some(pixel) = target.get_mut(at..at + 4) else {
                return;
            };
            let alpha = source >> 24;
            let inverse = 255 - alpha;
            let channels = [source & 0xFF, (source >> 8) & 0xFF, (source >> 16) & 0xFF];
            for (byte, channel) in pixel.iter_mut().zip(channels) {
                // Premultiplied: result = source + destination * (1 - alpha).
                let mixed = channel + (u32::from(*byte) * inverse + 127) / 255;
                *byte = mixed.min(255) as u8;
            }
            pixel[3] = 0xFF;
        }
    }
}
