//! The pointer metadata block of PipeWire (plan 2026-11-14, BK8a, B-16):
//! parser of `SPA_META_Cursor`, the pointer image, and its mixing into a
//! picture. Pure functions over bytes, so they run without PipeWire.
//!
//! Layouts (`spa/buffer/meta.h` of PipeWire master, read 2026-11-14 at
//! raw.githubusercontent.com/PipeWire/pipewire/master/spa/include/spa/buffer/meta.h;
//! the headers are not installed on this workstation, so the 0.3.x copies
//! that a host really loads are not compared [assumption: the layout is
//! stable over 0.3.x, the same assumption A11 as `pipewire_abi.rs`]).
//! All fields are native-endian (little-endian on every Linux target here):
//! ```text
//! struct spa_meta_cursor (28 bytes)       struct spa_meta_bitmap (20 bytes)
//!   u32 id          @0                      u32 format      @0  (0: no new bitmap)
//!   u32 flags       @4                      u32 width       @4  (spa_rectangle)
//!   i32 position.x  @8                      u32 height      @8
//!   i32 position.y  @12                     i32 stride      @12
//!   i32 hotspot.x   @16                     u32 offset      @16 (from the start of
//!   i32 hotspot.y   @20                                          this struct; 0: invisible)
//!   u32 bitmap_offset @24 (from the start of the cursor struct; 0: no new
//!                          bitmap; >= 28: a spa_meta_bitmap lies there)
//! ```
//! `id` 0 means no pointer (older headers: "invisible cursor"; master: "no new
//! cursor data"): it hides the pointer here, as the screen capture clients do
//! [assumption: the producers (mutter, KWin) send id 0 only to hide]. Bitmap
//! formats are `enum spa_video_format` values 7 to 14 (RGBx, BGRx, xRGB,
//! xBGR, RGBA, BGRA, ARGB, ABGR; `spa/param/video/raw.h`), named by memory
//! byte order [assumption: GStreamer's meaning]; their alpha is read as not
//! premultiplied [assumption: unverified, wrong premultiplication would show
//! as dark edges]. The position is the hot spot in pixels of the picture
//! [assumption: monitor streams; window streams unverified].
use super::FrameSize;

/// `SPA_META_Cursor` (`enum spa_meta_type`).
pub(in super::super) const SPA_META_CURSOR: u32 = super::super::super::spa_pod::META_CURSOR;
/// `sizeof(struct spa_meta_cursor)`.
pub(in super::super) const CURSOR_STRUCT: usize = 28;
/// `sizeof(struct spa_meta_bitmap)`.
const BITMAP_STRUCT: usize = 20;
/// Largest pointer image taken (the overlay's own limit,
/// `cursor_overlay::CursorShape::sampled`).
const MAX_SIDE: u32 = 512;
/// Side of the pointer image the meta size is requested for, and its largest.
const REQUEST_SIDE: u32 = 64;
const REQUEST_SIDE_MAX: u32 = 256;

fn meta_size(side: u32) -> u32 {
    (CURSOR_STRUCT + BITMAP_STRUCT) as u32 + side * side * 4
}

/// The `Meta` parameter the stream sends after the format is negotiated.
pub(in super::super) fn request_pod() -> Vec<u8> {
    super::super::super::spa_pod::cursor_meta_pod(
        meta_size(REQUEST_SIDE),
        meta_size(1),
        meta_size(REQUEST_SIDE_MAX),
    )
}

/// What a cursor block says about the bitmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) enum BitmapUpdate {
    /// No new bitmap: the shape held stays.
    Unchanged,
    /// A new image that is no pointer (invisible, too large, unknown format).
    Empty,
    /// A new image, `width * height` RGBA bytes with premultiplied colors.
    New {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// A parsed `spa_meta_cursor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct CursorMeta {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub hot_x: i32,
    pub hot_y: i32,
    pub bitmap: BitmapUpdate,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(at..end)?.try_into().ok()?))
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    u32_at(bytes, at).map(|value| value as i32)
}

/// Where the channels of a pixel lie in memory; `alpha` is `None` for the
/// formats with an unused byte.
struct Order {
    red: usize,
    green: usize,
    blue: usize,
    alpha: Option<usize>,
}

fn order(format: u32) -> Option<Order> {
    let (red, green, blue, alpha) = match format {
        7 => (0, 1, 2, None),     // RGBx
        8 => (2, 1, 0, None),     // BGRx
        9 => (1, 2, 3, None),     // xRGB
        10 => (3, 2, 1, None),    // xBGR
        11 => (0, 1, 2, Some(3)), // RGBA
        12 => (2, 1, 0, Some(3)), // BGRA
        13 => (1, 2, 3, Some(0)), // ARGB
        14 => (3, 2, 1, Some(0)), // ABGR
        _ => return None,
    };
    Some(Order {
        red,
        green,
        blue,
        alpha,
    })
}

fn premultiplied(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

/// The bitmap at `start` of the cursor block; `None` when the block is
/// malformed (a range outside it): the whole update is then ignored.
fn parse_bitmap(bytes: &[u8], start: usize) -> Option<BitmapUpdate> {
    if start < CURSOR_STRUCT {
        return None;
    }
    let field = |offset: usize| u32_at(bytes, start.checked_add(offset)?);
    let format = field(0)?;
    if format == 0 {
        return Some(BitmapUpdate::Unchanged);
    }
    let (width, height) = (field(4)?, field(8)?);
    let stride = field(12)? as i32;
    let offset = field(16)? as usize;
    if offset == 0 {
        return Some(BitmapUpdate::Empty);
    }
    if offset < BITMAP_STRUCT {
        return None;
    }
    let Some(order) = order(format) else {
        return Some(BitmapUpdate::Empty);
    };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Some(BitmapUpdate::Empty);
    }
    let (columns, rows) = (width as usize, height as usize);
    let stride = usize::try_from(stride).ok().filter(|s| *s >= columns * 4)?;
    let first = start.checked_add(offset)?;
    let span = stride.checked_mul(rows - 1)?.checked_add(columns * 4)?;
    let data = bytes.get(first..first.checked_add(span)?)?;
    let mut rgba = Vec::with_capacity(columns * rows * 4);
    for row in 0..rows {
        let line = &data[row * stride..row * stride + columns * 4];
        for pixel in line.chunks_exact(4) {
            let alpha = order.alpha.map_or(255, |at| pixel[at]);
            rgba.extend_from_slice(&[
                premultiplied(pixel[order.red], alpha),
                premultiplied(pixel[order.green], alpha),
                premultiplied(pixel[order.blue], alpha),
                alpha,
            ]);
        }
    }
    Some(BitmapUpdate::New {
        width,
        height,
        rgba,
    })
}

/// Parses the bytes of one `SPA_META_Cursor` block (`None`: malformed).
pub(in super::super) fn parse_cursor_meta(bytes: &[u8]) -> Option<CursorMeta> {
    let id = u32_at(bytes, 0)?;
    let bitmap_offset = u32_at(bytes, 24)? as usize;
    let bitmap = if bitmap_offset == 0 {
        BitmapUpdate::Unchanged
    } else {
        parse_bitmap(bytes, bitmap_offset)?
    };
    Some(CursorMeta {
        id,
        x: i32_at(bytes, 8)?,
        y: i32_at(bytes, 12)?,
        hot_x: i32_at(bytes, 16)?,
        hot_y: i32_at(bytes, 20)?,
        bitmap,
    })
}

/// The pointer's image, full size, with its hot spot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct PointerBitmap {
    pub width: u32,
    pub height: u32,
    pub hot_x: i32,
    pub hot_y: i32,
    /// RGBA bytes, colors premultiplied.
    pub rgba: Vec<u8>,
}

/// Mixes `bitmap` onto the BGRA `target` of `size`, its hot spot at
/// (`x`, `y`). Premultiplied: result = source + destination * (1 - alpha).
pub(in super::super) fn blend_premultiplied(
    target: &mut [u8],
    size: FrameSize,
    bitmap: &PointerBitmap,
    x: i32,
    y: i32,
) {
    let (width, height) = (i64::from(size.width), i64::from(size.height));
    let left = i64::from(x) - i64::from(bitmap.hot_x);
    let top = i64::from(y) - i64::from(bitmap.hot_y);
    for row in 0..i64::from(bitmap.height) {
        let at_y = top + row;
        if !(0..height).contains(&at_y) {
            continue;
        }
        for column in 0..i64::from(bitmap.width) {
            let at_x = left + column;
            if !(0..width).contains(&at_x) {
                continue;
            }
            let from = ((row * i64::from(bitmap.width) + column) * 4) as usize;
            let Some(source) = bitmap.rgba.get(from..from + 4) else {
                return;
            };
            let to = ((at_y * width + at_x) * 4) as usize;
            let Some(pixel) = target.get_mut(to..to + 4) else {
                return;
            };
            let inverse = 255 - u32::from(source[3]);
            for (byte, channel) in pixel.iter_mut().zip([source[2], source[1], source[0]]) {
                let mixed = u32::from(channel) + (u32::from(*byte) * inverse + 127) / 255;
                *byte = mixed.min(255) as u8;
            }
            pixel[3] = 0xFF;
        }
    }
}
