//! Lossless tile codec `tiles` (decision C2 L2). The frame is cut into 64×64
//! tiles; a key frame carries every tile, a delta frame only the tiles whose
//! 64-bit hash changed. A tile travels as one colour, as raw BGRA or as an
//! LZ4 block (`lz4_flex`), whichever is smallest. A frame becomes records of
//! at most 4 MiB (`MAX_TILES_RECORD_BYTES`); they travel as frame records of
//! one picture stream with `FLAG_MORE` on every record but the last.
//!
//! Record: tile count `u16`, then per tile `x u16, y u16, w u8, h u8,
//! encoding u8, length u32` and the data, all big-endian. Alpha is always
//! 255: screen content is opaque, and BGRx sources leave the fourth byte
//! undefined.
use std::io;

use lz4_flex::block::{compress_into, decompress_into, get_maximum_output_size};

use super::pixel_convert::{BgraImage, BgraView, FrameSize, Rect};
use super::wire::{invalid, BeReader};

pub(crate) const TILE_SIDE: u32 = 64;
/// Largest record of a tile frame (C2).
pub(crate) const MAX_TILES_RECORD_BYTES: usize = 4 * 1024 * 1024;
const TILE_HEADER_BYTES: usize = 11;
const RECORD_HEADER_BYTES: usize = 2;
const MAX_TILE_BYTES: usize = (TILE_SIDE * TILE_SIDE * 4) as usize;
/// The largest encoded tile: its header plus a raw tile.
pub(crate) const MAX_ENCODED_TILE_BYTES: usize = TILE_HEADER_BYTES + MAX_TILE_BYTES;
/// Smallest record limit an encoder accepts: one worst-case tile.
pub(crate) const MIN_RECORD_LIMIT: usize = RECORD_HEADER_BYTES + MAX_ENCODED_TILE_BYTES;

const ENCODING_SOLID: u8 = 0;
const ENCODING_RAW: u8 = 1;
const ENCODING_LZ4: u8 = 2;

/// Alpha bytes of two little-endian BGRA pixels in one `u64`.
const ALPHA_MASK: u64 = 0xFF00_0000_FF00_0000;

pub(super) fn tile_rect(column: u32, row: u32, size: FrameSize) -> Rect {
    let (x, y) = (column * TILE_SIDE, row * TILE_SIDE);
    Rect {
        x,
        y,
        width: TILE_SIDE.min(size.width - x),
        height: TILE_SIDE.min(size.height - y),
    }
}

/// Hash of the colour bytes of one tile. Every step is a bijection of the
/// state for a fixed input word, so two tiles that differ in exactly one
/// 8-byte word never share a hash.
pub(super) fn tile_hash(frame: &BgraView, rect: Rect) -> u64 {
    let mut state = 0x243F_6A88_85A3_08D3u64;
    let (start, end) = (rect.x as usize * 4, (rect.x + rect.width) as usize * 4);
    for y in rect.y..rect.y + rect.height {
        let line = &frame.row(y)[start..end];
        let mut words = line.chunks_exact(8);
        for word in &mut words {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(word);
            state = mix(state, u64::from_le_bytes(bytes) & !ALPHA_MASK);
        }
        if let [b, g, r, _] = *words.remainder() {
            state = mix(state, u64::from_le_bytes([b, g, r, 0, 0, 0, 0, 1]));
        }
    }
    state
}

fn mix(state: u64, word: u64) -> u64 {
    (state ^ word)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(29)
}

/// Copies one tile into `out` as tight rows with alpha 255.
fn gather(frame: &BgraView, rect: Rect, out: &mut Vec<u8>) {
    out.clear();
    let (start, end) = (rect.x as usize * 4, (rect.x + rect.width) as usize * 4);
    for y in rect.y..rect.y + rect.height {
        out.extend_from_slice(&frame.row(y)[start..end]);
    }
    for pixel in out.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
}

pub(super) fn solid_colour(tile: &[u8]) -> Option<[u8; 4]> {
    let first: [u8; 4] = tile.get(..4)?.try_into().ok()?;
    tile.chunks_exact(4)
        .all(|pixel| pixel == first)
        .then_some(first)
}

/// Appends one encoded tile to `record`.
fn append_tile(record: &mut Vec<u8>, rect: Rect, tile: &[u8]) -> io::Result<()> {
    record.extend_from_slice(&(rect.x as u16).to_be_bytes());
    record.extend_from_slice(&(rect.y as u16).to_be_bytes());
    record.extend_from_slice(&[rect.width as u8, rect.height as u8]);
    let meta = record.len();
    record.extend_from_slice(&[0u8; 5]);
    let (encoding, len) = if let Some(colour) = solid_colour(tile) {
        record.extend_from_slice(&colour);
        (ENCODING_SOLID, 4)
    } else {
        let data = record.len();
        record.resize(data + get_maximum_output_size(tile.len()), 0);
        let written = compress_into(tile, &mut record[data..])
            .map_err(|_| invalid("LZ4 compression of a screen tile failed"))?;
        if written < tile.len() {
            record.truncate(data + written);
            (ENCODING_LZ4, written)
        } else {
            record.truncate(data);
            record.extend_from_slice(tile);
            (ENCODING_RAW, tile.len())
        }
    };
    record[meta] = encoding;
    record[meta + 1..meta + 5].copy_from_slice(&(len as u32).to_be_bytes());
    Ok(())
}

/// Host side: remembers the hash of every tile of the last frame.
#[derive(Clone, Debug)]
pub(crate) struct TileEncoder {
    size: FrameSize,
    columns: u32,
    rows: u32,
    hashes: Vec<u64>,
    primed: bool,
    tile: Vec<u8>,
    previous: Option<BgraImage>,
    limits: super::frame_budget::FrameLimits,
}

impl TileEncoder {
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        Self::new_with_limits(size, super::frame_budget::FrameLimits::LEGACY)
    }

    pub(crate) fn new_with_limits(size: FrameSize, limits: super::frame_budget::FrameLimits) -> io::Result<Self> {
        limits.check(size)?;
        let (columns, rows) = (
            size.width.div_ceil(TILE_SIDE),
            size.height.div_ceil(TILE_SIDE),
        );
        Ok(Self {
            size,
            columns,
            rows,
            hashes: vec![0; columns as usize * rows as usize],
            primed: false,
            tile: Vec::with_capacity(MAX_TILE_BYTES),
            previous: None, limits,
        })
    }

    pub(crate) fn size(&self) -> FrameSize {
        self.size
    }

    /// Encodes `frame` into records of at most `record_limit` bytes; the
    /// first frame is always a key frame. No records means nothing changed.
    /// The hashes advance with every call, so after a frame that did not
    /// reach the viewer (reset stream, send error) the next call must be a
    /// key frame; the pacer ensures that (`on_stream_reset`).
    pub(crate) fn encode(
        &mut self,
        frame: &BgraView,
        keyframe: bool,
        record_limit: usize,
    ) -> io::Result<Vec<Vec<u8>>> {
        self.encode_visible(frame,keyframe,record_limit,None)
    }
    /// Full references rebuild all pixels; deltas retain unsent pixels and hashes.
    pub(crate) fn encode_visible(&mut self, frame: &BgraView, keyframe: bool, record_limit: usize,
        visible: Option<&[super::visibility::VisibleRect]>) -> io::Result<Vec<Vec<u8>>> {
        if frame.size() != self.size {
            return Err(invalid("tile frame size differs from the encoder size"));
        }
        if !(MIN_RECORD_LIMIT..=MAX_TILES_RECORD_BYTES).contains(&record_limit) {
            return Err(invalid("tile record limit is outside 16 KiB..=4 MiB"));
        }
        let full = keyframe || !self.primed;
        let mut previous=self.previous.take().map(Ok).unwrap_or_else(||BgraImage::new_with_limits(self.size,self.limits))?;
        let mut records: Vec<Vec<u8>> = Vec::new();
        let mut count = 0u16;
        for row in 0..self.rows {
            for column in 0..self.columns {
                let rect = tile_rect(column, row, self.size);
                if !full && !super::frame_regions::visible(rect,visible) {continue;}
                let hash = tile_hash(frame, rect);
                let slot = &mut self.hashes[(row * self.columns + column) as usize];
                let exact=!full && *slot==hash && {
                    let from=rect.x as usize*4; let end=(rect.x+rect.width) as usize*4;
                    (rect.y..rect.y+rect.height).all(|y| previous.view().row(y)[from..end].chunks_exact(4)
                        .zip(frame.row(y)[from..end].chunks_exact(4)).all(|(a,b)|a[..3]==b[..3]))
                };
                if !full && *slot == hash && exact { continue; }
                *slot = hash;
                let fits = records.last().is_some_and(|record| {
                    record.len() + MAX_ENCODED_TILE_BYTES <= record_limit && count < u16::MAX
                });
                if !fits {
                    close(&mut records, count);
                    records.push(vec![0u8; RECORD_HEADER_BYTES]);
                    count = 0;
                }
                gather(frame, rect, &mut self.tile);
                if let Some(record) = records.last_mut() {
                    append_tile(record, rect, &self.tile)?;
                    count += 1;
                    let from=rect.x as usize*4;let end=(rect.x+rect.width) as usize*4;
                    for y in rect.y..rect.y+rect.height {
                        let row=&mut previous.row_mut(y)[from..end];row.copy_from_slice(&frame.row(y)[from..end]);
                        for pixel in row.chunks_exact_mut(4) {pixel[3]=255;}
                    }
                }
            }
        }
        close(&mut records, count);
        self.previous=Some(previous);
        self.primed = true;
        Ok(records)
    }
}

/// Writes the tile count into the last record.
fn close(records: &mut [Vec<u8>], count: u16) {
    if let Some(record) = records.last_mut() {
        record[..RECORD_HEADER_BYTES].copy_from_slice(&count.to_be_bytes());
    }
}

/// Viewer side: the picture the records have built so far.
#[derive(Clone, Debug)]
pub(crate) struct TileDecoder {
    image: BgraImage,
    scratch: Vec<u8>,
    frame_id: Option<u64>,
    coverage: Vec<bool>,
    last_rects: Vec<Rect>,
}

impl TileDecoder {
    /// An opaque black picture of `size` (from the picture stream head).
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            image: BgraImage::new(size)?,
            scratch: Vec::with_capacity(MAX_TILE_BYTES),
            frame_id: None, coverage: Vec::new(), last_rects: Vec::new(),
        })
    }

    pub(crate) fn new_with_limits(size: FrameSize, limits: super::frame_budget::FrameLimits) -> io::Result<Self> {
        Ok(Self { image: BgraImage::new_with_limits(size, limits)?, scratch: Vec::with_capacity(MAX_TILE_BYTES), frame_id:None,coverage:Vec::new(),last_rects:Vec::new() })
    }

    pub(crate) fn apply_frame_record(&mut self, record: &super::wire_binary::FrameRecord) -> io::Result<bool> {
        if self.frame_id != Some(record.frame_id) {
            self.frame_id=Some(record.frame_id);
            self.coverage=vec![false; self.image.size().width.div_ceil(TILE_SIDE) as usize * self.image.size().height.div_ceil(TILE_SIDE) as usize];
        }
        self.apply_record(&record.payload)?;
        let columns=self.image.size().width.div_ceil(TILE_SIDE);
        for rect in &self.last_rects { self.coverage[((rect.y/TILE_SIDE)*columns+rect.x/TILE_SIDE) as usize]=true; }
        if !record.has_more() && record.is_keyframe() && self.coverage.iter().any(|covered| !covered) {
            return Err(invalid("lossless keyframe did not rebuild every tile"));
        }
        Ok(!record.has_more())
    }

    pub(crate) fn image(&self) -> &BgraImage {
        &self.image
    }

    /// Applies one record and returns its tile count. After an error the
    /// picture may be partly updated; the viewer then asks for a key frame.
    pub(crate) fn apply_record(&mut self, record: &[u8]) -> io::Result<usize> {
        if record.len() > MAX_TILES_RECORD_BYTES {
            return Err(invalid("tile record exceeds 4 MiB"));
        }
        self.last_rects.clear();
        let size = self.image.size();
        let mut reader = BeReader::new(record);
        let count = reader.u16()?;
        for _ in 0..count {
            let x = u32::from(reader.u16()?);
            let y = u32::from(reader.u16()?);
            let (width, height) = (u32::from(reader.u8()?), u32::from(reader.u8()?));
            let encoding = reader.u8()?;
            let len = reader.u32()? as usize;
            let rect = Rect {
                x,
                y,
                width,
                height,
            };
            let on_grid = x.is_multiple_of(TILE_SIDE)
                && y.is_multiple_of(TILE_SIDE)
                && x < size.width
                && y < size.height;
            if !on_grid || tile_rect(x / TILE_SIDE, y / TILE_SIDE, size) != rect {
                return Err(invalid(
                    "screen tile lies off the 64-pixel grid of the frame",
                ));
            }
            self.last_rects.push(rect);
            let raw_len = (width * height * 4) as usize;
            let data = reader.bytes(len)?;
            match encoding {
                ENCODING_SOLID if len == 4 => self.fill(rect, data),
                ENCODING_RAW if len == raw_len => self.copy(rect, data),
                ENCODING_LZ4 if len < raw_len => {
                    self.scratch.resize(raw_len, 0);
                    let written = decompress_into(data, &mut self.scratch)
                        .map_err(|_| invalid("LZ4 data of a screen tile is corrupt"))?;
                    if written != raw_len {
                        return Err(invalid("LZ4 data of a screen tile has the wrong length"));
                    }
                    let tile = std::mem::take(&mut self.scratch);
                    self.copy(rect, &tile);
                    self.scratch = tile;
                }
                _ => return Err(invalid("unknown encoding or length of a screen tile")),
            }
        }
        reader.finish()?;
        Ok(usize::from(count))
    }

    fn fill(&mut self, rect: Rect, colour: &[u8]) {
        for y in rect.y..rect.y + rect.height {
            let row = self.image.row_mut(y);
            for pixel in
                row[rect.x as usize * 4..(rect.x + rect.width) as usize * 4].chunks_exact_mut(4)
            {
                pixel.copy_from_slice(colour);
            }
        }
    }

    fn copy(&mut self, rect: Rect, tile: &[u8]) {
        let line = rect.width as usize * 4;
        for (offset, source) in tile.chunks_exact(line).enumerate() {
            let row = self.image.row_mut(rect.y + offset as u32);
            row[rect.x as usize * 4..rect.x as usize * 4 + line].copy_from_slice(source);
        }
    }
}

#[cfg(test)]
#[path = "tiles_codec_tests.rs"]
mod tests;
