//! Still pictures (SCR-M4, `se screen snapshot`, MCP `screen` action
//! `snapshot`): a lossless PNG of one BGRA picture, a size that fits a long
//! edge and a byte budget, and the picture rebuilt from the tile records of
//! a picture stream of the kind `snapshot`. Platform-neutral: the capture
//! lives in `os/`, the transport in the daemon.
use std::io;

use super::pixel_convert::{downscale_bgra, BgraImage, FrameSize};
use super::tiles_codec::TileDecoder;
use super::wire_binary::FrameRecord;

/// Below this long edge a picture is no longer shrunk to fit a byte budget:
/// 64 pixels is one tile of the tile codec and shows nothing readable.
pub(crate) const MIN_FIT_EDGE: u32 = 64;

/// A PNG and the size of the picture it holds.
#[derive(Debug)]
pub(crate) struct Fitted {
    pub png: Vec<u8>,
    pub size: FrameSize,
}

fn invalid_input(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text.to_string())
}

/// `image` as a lossless 8-bit RGB PNG; the alpha channel of a screen
/// picture carries nothing and is dropped.
pub(crate) fn encode_png(image: &BgraImage) -> io::Result<Vec<u8>> {
    let size = image.size();
    let mut rgb = Vec::with_capacity(size.width as usize * size.height as usize * 3);
    for pixel in image.data().chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size.width, size.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::default());
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&rgb).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    Ok(out)
}

/// The PNG of `image` at `size` (the picture itself when the size equals).
fn png_at(image: &BgraImage, size: FrameSize) -> io::Result<Vec<u8>> {
    if image.size() == size {
        return encode_png(image);
    }
    encode_png(&downscale_bgra(&image.view(), size)?)
}

fn long_edge(size: FrameSize) -> u32 {
    size.width.max(size.height)
}

/// The PNG of `image` within `max_edge` on its long edge and, when
/// `max_bytes` is set, within that many bytes: a PNG that is too large is
/// made again from the original at 3/4 of the edges until it fits (never
/// from an already shrunk picture, so no detail is lost twice). Below
/// `MIN_FIT_EDGE` pixels it gives up.
pub(crate) fn fit(
    image: &BgraImage,
    max_edge: Option<u32>,
    max_bytes: Option<usize>,
) -> io::Result<Fitted> {
    let mut size = match max_edge {
        Some(0) => return Err(invalid_input("max_edge must be at least 1")),
        Some(edge) => image.size().fit_within(FrameSize::new(edge, edge)),
        None => image.size(),
    };
    loop {
        let png = png_at(image, size)?;
        let fits = match max_bytes {
            Some(limit) => png.len() <= limit,
            None => true,
        };
        if fits {
            return Ok(Fitted { png, size });
        }
        let next = size.scale(3, 4);
        if long_edge(next) < MIN_FIT_EDGE || next == size {
            return Err(invalid_input("Das Standbild passt nicht in max_bytes"));
        }
        size = next;
    }
}

/// The picture of a `snapshot` stream: its tile records, in order, applied
/// to a black picture of the stream's `size`. The last record must end the
/// frame.
pub(crate) fn decode_snapshot(records: &[FrameRecord], size: FrameSize) -> io::Result<BgraImage> {
    let Some(last) = records.last() else {
        return Err(invalid_input("the snapshot has no records"));
    };
    if last.has_more() {
        return Err(invalid_input("the snapshot ends in the middle of a frame"));
    }
    let mut decoder = TileDecoder::new(size)?;
    for record in records {
        decoder.apply_record(&record.payload)?;
    }
    Ok(decoder.image().clone())
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
