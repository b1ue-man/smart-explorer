//! Test source `synthetic` (decision C9): a deterministic picture per frame
//! id with a barcode (top left), rolling text (middle band), a static text
//! block (bottom left, the region of `static_psnr_db`) and a moving
//! rectangle (bottom right). The barcode holds the frame id (64 bits) and the
//! CRC-32 of its 8 big-endian bytes (32 bits) in 16 × 6 black or white cells
//! large enough to survive lossy coding; a read whose checksum fails counts
//! as a corrupt frame. The source shows no screen content (C7).
use std::io;

use super::pixel_convert::{BgraImage, BgraView, FrameSize, Rect};
use super::wire::invalid;

/// Smallest synthetic frame: the barcode needs 16 cells of 4 pixels plus
/// margins, the text block 64 × 24 pixels below it.
pub(crate) const MIN_SYNTHETIC_SIZE: FrameSize = FrameSize::new(128, 72);
const BARCODE_COLUMNS: u32 = 16;
const BARCODE_ROWS: u32 = 6;
const BACKGROUND: [u8; 4] = [0x30, 0x30, 0x30, 0xFF];
const INK: [u8; 4] = [0xF0, 0xF0, 0xF0, 0xFF];
const RECTANGLE: [u8; 4] = [0x20, 0x90, 0xE0, 0xFF];
const BLACK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const STATIC_LINES: [&str; 4] = [
    "0123456789ABCDEF",
    "FEDCBA9876543210",
    "02468ACE13579BDF",
    "F0E1D2C3B4A59687",
];

/// Hex digits 0–F in 3 × 5 pixels, one row of 3 bits per entry (top first).
const GLYPHS: [[u8; 5]; 16] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
    [2, 5, 7, 5, 5],
    [6, 5, 6, 5, 6],
    [7, 4, 4, 4, 7],
    [6, 5, 5, 5, 6],
    [7, 4, 7, 4, 7],
    [7, 4, 7, 4, 4],
];

/// Placement of every element for one frame size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    cell: u32,
    margin: u32,
    scale: u32,
}

impl Layout {
    fn of(size: FrameSize) -> Self {
        let cell = (size.width.min(size.height) / 36).clamp(4, 32);
        Self {
            cell,
            margin: cell,
            scale: (size.height / 180).max(1),
        }
    }

    fn text_block(self, size: FrameSize) -> Rect {
        Rect {
            x: self.margin,
            y: size.height - self.margin - 24 * self.scale,
            width: 64 * self.scale,
            height: 24 * self.scale,
        }
    }
}

/// The static text block, the region of `static_psnr_db`.
pub(crate) fn static_block(size: FrameSize) -> Rect {
    Layout::of(size).text_block(size)
}

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320).
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn barcode_bits(frame_id: u64) -> u128 {
    let crc = crc32(&frame_id.to_be_bytes());
    (u128::from(frame_id) << 32) | u128::from(crc)
}

pub(crate) trait SyntheticCanvas {
    fn size(&self) -> FrameSize;
    fn pixels(&mut self) -> &mut [u8];
    fn row(&mut self, y: u32) -> Option<&mut [u8]>;
}
impl SyntheticCanvas for BgraImage {
    fn size(&self) -> FrameSize {self.size()}
    fn pixels(&mut self) -> &mut [u8] {self.data_mut()}
    fn row(&mut self, y: u32) -> Option<&mut [u8]> {
        if y < self.size().height {Some(self.row_mut(y))} else {None}
    }
}
impl SyntheticCanvas for super::capture_image::CaptureImageBuilder {
    fn size(&self) -> FrameSize {self.size()}
    fn pixels(&mut self) -> &mut [u8] {self.pixels_mut()}
    fn row(&mut self, y: u32) -> Option<&mut [u8]> {self.row_mut(y)}
}
fn fill(image: &mut impl SyntheticCanvas, x0: i64, y0: i64, width: u32, height: u32, colour: [u8; 4]) {
    let size = image.size();
    let left = x0.clamp(0, i64::from(size.width)) as usize;
    let right = (x0 + i64::from(width)).clamp(0, i64::from(size.width)) as usize;
    let top = y0.clamp(0, i64::from(size.height)) as u32;
    let bottom = (y0 + i64::from(height)).clamp(0, i64::from(size.height)) as u32;
    for y in top..bottom {
        let Some(row) = image.row(y) else {continue;};
        for pixel in row[left * 4..right * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&colour);
        }
    }
}

fn draw_text(image: &mut impl SyntheticCanvas, text: &str, x: i64, y: i64, scale: u32) {
    for (index, character) in text.chars().enumerate() {
        let Some(glyph) = character.to_digit(16).map(|digit| GLYPHS[digit as usize]) else {
            continue;
        };
        let left = x + index as i64 * 4 * i64::from(scale);
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..3u32 {
                let mask = 4u8 >> column;
                if bits & mask != 0 {
                    let px = left + i64::from(column * scale);
                    let py = y + row as i64 * i64::from(scale);
                    fill(image, px, py, scale, scale, INK);
                }
            }
        }
    }
}

/// Draws the picture of `frame_id` into `image` (at least 128 × 72).
pub(crate) fn render(frame_id: u64, image: &mut impl SyntheticCanvas) -> io::Result<()> {
    let size = image.size();
    if size.width < MIN_SYNTHETIC_SIZE.width || size.height < MIN_SYNTHETIC_SIZE.height {
        return Err(invalid("synthetic frames need at least 128 x 72 pixels"));
    }
    let layout = Layout::of(size);
    for pixel in image.pixels().chunks_exact_mut(4) {
        pixel.copy_from_slice(&BACKGROUND);
    }
    let bits = barcode_bits(frame_id);
    for index in 0..BARCODE_COLUMNS * BARCODE_ROWS {
        let set = (bits >> (95 - index)) & 1 == 1;
        let x = layout.margin + (index % BARCODE_COLUMNS) * layout.cell;
        let y = layout.margin + (index / BARCODE_COLUMNS) * layout.cell;
        let colour = if set { WHITE } else { BLACK };
        fill(
            image,
            i64::from(x),
            i64::from(y),
            layout.cell,
            layout.cell,
            colour,
        );
    }
    let scale = layout.scale;
    let text_y = i64::from(size.height / 2) - 3 * i64::from(scale);
    let line = format!("{frame_id:016X}{frame_id:016X}");
    let span = 16 * 4 * i64::from(scale);
    let shift = (frame_id % 64) as i64 * i64::from(scale);
    let mut x = -shift;
    while x < i64::from(size.width) {
        draw_text(image, &line, x, text_y, scale);
        x += 2 * span;
    }
    let block = layout.text_block(size);
    for (row, text) in STATIC_LINES.iter().enumerate() {
        let y = i64::from(block.y) + row as i64 * 6 * i64::from(scale);
        draw_text(image, text, i64::from(block.x), y, scale);
    }
    let left = (block.x + block.width + layout.margin).max(size.width / 2);
    let room = size
        .width
        .saturating_sub(layout.margin)
        .saturating_sub(left);
    let (width, height) = (
        (size.width / 8).min(room / 2).max(1),
        (size.height / 8).max(1),
    );
    let travel = u64::from(room.saturating_sub(width).max(1));
    let step = (frame_id % (2 * travel)) * 4 % (2 * travel);
    let offset = if step < travel {
        step
    } else {
        2 * travel - step
    };
    let rect_x = i64::from(left) + offset as i64;
    let rect_y = i64::from(size.height - layout.margin - height);
    fill(image, rect_x, rect_y, width, height, RECTANGLE);
    Ok(())
}

/// Result of reading the barcode of a received frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarcodeRead {
    Valid(u64),
    /// The checksum does not match: a corrupt or mixed frame.
    Corrupt,
}

/// Mean luma of a region (BT.601 weights, 8 fractional bits).
fn mean_luma(image: &BgraView, left: u32, top: u32, right: u32, bottom: u32) -> u32 {
    let (mut sum, mut count) = (0u64, 0u64);
    for y in top..bottom {
        for pixel in image.row(y)[left as usize * 4..right as usize * 4].chunks_exact(4) {
            let (b, g, r) = (
                u64::from(pixel[0]),
                u64::from(pixel[1]),
                u64::from(pixel[2]),
            );
            sum += (77 * r + 150 * g + 29 * b) >> 8;
            count += 1;
        }
    }
    (sum / count.max(1)) as u32
}

/// Reads the barcode of `image`, a frame of a synthetic source of size
/// `source` that may have been scaled on the way (resolution ladder).
pub(crate) fn read_barcode(image: &BgraView, source: FrameSize) -> io::Result<BarcodeRead> {
    let size = image.size();
    if source.width < MIN_SYNTHETIC_SIZE.width || source.height < MIN_SYNTHETIC_SIZE.height {
        return Err(invalid("synthetic source size is below 128 x 72 pixels"));
    }
    let layout = Layout::of(source);
    let map = |value: u32, from: u32, to: u32| {
        (u64::from(value) * u64::from(to) / u64::from(from)) as u32
    };
    let mut bits = 0u128;
    for index in 0..BARCODE_COLUMNS * BARCODE_ROWS {
        let x = layout.margin + (index % BARCODE_COLUMNS) * layout.cell;
        let y = layout.margin + (index / BARCODE_COLUMNS) * layout.cell;
        let quarter = layout.cell / 4;
        let left = map(x + quarter, source.width, size.width);
        let top = map(y + quarter, source.height, size.height);
        let right = map(x + layout.cell - quarter, source.width, size.width)
            .max(left + 1)
            .min(size.width);
        let bottom = map(y + layout.cell - quarter, source.height, size.height)
            .max(top + 1)
            .min(size.height);
        if left >= right || top >= bottom {
            return Err(invalid("frame is too small to hold the barcode"));
        }
        let set = mean_luma(image, left, top, right, bottom) >= 128;
        bits = (bits << 1) | u128::from(set);
    }
    let frame_id = (bits >> 32) as u64;
    let crc = (bits & u128::from(u32::MAX)) as u32;
    Ok(if crc32(&frame_id.to_be_bytes()) == crc {
        BarcodeRead::Valid(frame_id)
    } else {
        BarcodeRead::Corrupt
    })
}

#[cfg(test)]
#[path = "synthetic_tests.rs"]
mod tests;
