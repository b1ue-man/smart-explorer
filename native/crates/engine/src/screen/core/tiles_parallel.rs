//! The lossless tile encoder on all cores (plan 2026-11-17 P9). Until then
//! `AdmittedTilesEncoder` hashed, compared, gathered and compressed every
//! tile on one thread: 24.9 ms for a 2880 × 1800 picture under full-screen
//! motion (measured 2026-11-17). The work now runs in two phases on rayon's
//! fixed pool (one thread per core), and the wire output stays byte for byte
//! that of the sequential encoder:
//!
//! 1. `look`: one task per row of tiles (a band of 64 pixel rows, so every
//!    task owns its rows of the previous picture, its dirty marks and its
//!    change flags). A tile that is visible and marked is compared with the
//!    previous picture; a changed one is copied into it with alpha 255.
//! 2. `encode`: the changed tiles in row-major order, in waves of at most
//!    `TILE_WAVE` tiles. Every tile of a wave is gathered from the previous
//!    picture into its own admitted slot and becomes one colour, an LZ4
//!    block or raw BGRA, whichever is smallest (the rule of `tiles_codec`);
//!    then the calling thread hands the wave's tiles to `emit` in order.
//!
//! The 64-bit tile hash is gone from this encoder: it was the hash of the
//! previous picture's tile, so "same hash and same pixels" is "same pixels",
//! and the direct comparison stops at the first differing word where the
//! hash read the whole tile. The comparison masks alpha as the hash did.
//!
//! Compression: LZ4 block (`lz4_flex` 0.14) stays. It is the encoding the
//! wire and every viewer know (`ENCODING_LZ4`); no faster lossless block
//! compressor is a dependency (no zstd in `native/Cargo.lock`), and LZ4 has
//! no level to lower: its block encoder has one speed.
//!
//! Target, an assumption that no measurement here backs: 24.9 ms on one
//! thread over eight cores is ≈ 3.1 ms; the order-keeping hand-over copies
//! only the encoded bytes on one thread.
use std::io;

use lz4_flex::block::compress_into;
use rayon::prelude::*;

use crate::screen::core::frame_regions::visible;
use crate::screen::core::pixel_convert::{BgraView, FrameSize, Rect};
use crate::screen::core::tiles_codec::{
    solid_colour, tile_rect, ENCODING_LZ4, ENCODING_RAW, ENCODING_SOLID, TILE_SIDE,
    TILE_SLOT_BYTES, TILE_WAVE,
};
use crate::screen::core::tiles_dirty::DirtyTiles;
use crate::screen::core::visibility::VisibleRect;
use crate::screen::core::wire::invalid;

/// Bytes of a raw 64 × 64 tile; the first part of every slot.
const RAW_TILE_BYTES: usize = (TILE_SIDE * TILE_SIDE * 4) as usize;
/// Alpha bytes of two little-endian BGRA pixels in one `u64`.
const ALPHA_MASK: u64 = 0xFF00_0000_FF00_0000;
/// Below this many tiles to look at, the calling thread looks alone: waking
/// the pool costs more than comparing a few tiles (typing touches 1–4).
const PARALLEL_MIN_TILES: usize = 8;

/// Where the work runs. The output is the same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workers {
    /// rayon's pool, one thread per core.
    Pool,
    /// The calling thread only (the sequential reference).
    Caller,
}

/// The picture to look at.
#[derive(Clone, Copy, Debug)]
pub struct Look<'a> {
    pub view: BgraView<'a>,
    /// A key frame: every tile, visible or not, changed or not.
    pub full: bool,
    pub visible: Option<&'a [VisibleRect]>,
}

/// How one tile of a wave was encoded.
#[derive(Clone, Copy, Debug, Default)]
struct Encoded {
    encoding: u8,
    len: usize,
}

/// The per-picture bookkeeping of the parallel encoder; every buffer is
/// sized once for its picture size and reused.
#[derive(Debug)]
pub struct TileWork {
    size: FrameSize,
    columns: u32,
    /// The tiles `look` found changed (row-major).
    changed: Vec<bool>,
    /// The tiles of the current wave.
    wave: Vec<Rect>,
    encoded: Vec<Encoded>,
}

impl TileWork {
    pub fn new(size: FrameSize) -> Self {
        let columns = size.width.div_ceil(TILE_SIDE);
        let rows = size.height.div_ceil(TILE_SIDE);
        Self {
            size,
            columns,
            changed: vec![false; columns as usize * rows as usize],
            wave: Vec::with_capacity(TILE_WAVE),
            encoded: Vec::with_capacity(TILE_WAVE),
        }
    }

    /// Phase 1: compares the visible, marked tiles of `look` with
    /// `previous` (the tight BGRA rows of the previous picture), clears
    /// their marks and copies the changed ones into `previous` with alpha
    /// 255. Returns how many tiles changed.
    pub fn look(
        &mut self,
        previous: &mut [u8],
        look: &Look<'_>,
        dirty: &mut DirtyTiles,
        workers: Workers,
    ) -> io::Result<usize> {
        if look.view.size() != self.size || dirty.size() != self.size {
            return Err(invalid("tile picture size differs from the encoder size"));
        }
        let row_bytes = self.size.width as usize * 4;
        if previous.len() != row_bytes * self.size.height as usize {
            return Err(invalid("previous tile picture has the wrong length"));
        }
        let band_bytes = row_bytes * TILE_SIDE as usize;
        let columns = self.columns as usize;
        let marks = dirty.marks_mut();
        let candidates = if look.full {
            marks.len()
        } else {
            marks.iter().filter(|mark| **mark).count()
        };
        let band = Band {
            look,
            size: self.size,
            row_bytes,
        };
        if workers == Workers::Pool && candidates >= PARALLEL_MIN_TILES {
            previous
                .par_chunks_mut(band_bytes)
                .zip(marks.par_chunks_mut(columns))
                .zip(self.changed.par_chunks_mut(columns))
                .enumerate()
                .for_each(|(row, ((pixels, marks), changed))| {
                    band.look(row as u32, pixels, marks, changed)
                });
        } else {
            previous
                .chunks_mut(band_bytes)
                .zip(marks.chunks_mut(columns))
                .zip(self.changed.chunks_mut(columns))
                .enumerate()
                .for_each(|(row, ((pixels, marks), changed))| {
                    band.look(row as u32, pixels, marks, changed)
                });
        }
        Ok(self.changed.iter().filter(|changed| **changed).count())
    }

    /// Phase 2: encodes the tiles `look` found changed from `previous`
    /// into the slots of `scratch` (`TILE_SLOT_BYTES` each; their count is
    /// the wave size) and hands each to `emit` in row-major order as
    /// (tile, encoding, data).
    pub fn encode(
        &mut self,
        previous: BgraView<'_>,
        scratch: &mut [u8],
        workers: Workers,
        mut emit: impl FnMut(Rect, u8, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if previous.size() != self.size {
            return Err(invalid("previous tile picture size differs"));
        }
        let slots = scratch.len() / TILE_SLOT_BYTES;
        if slots == 0 {
            return Err(invalid("tile encoder has no slot"));
        }
        let mut next = 0usize;
        loop {
            self.wave.clear();
            while self.wave.len() < slots && next < self.changed.len() {
                if self.changed[next] {
                    let index = next as u32;
                    let (column, row) = (index % self.columns, index / self.columns);
                    self.wave.push(tile_rect(column, row, self.size));
                }
                next += 1;
            }
            if self.wave.is_empty() {
                return Ok(());
            }
            let count = self.wave.len();
            let work = &mut scratch[..count * TILE_SLOT_BYTES];
            self.encoded.clear();
            self.encoded.resize(count, Encoded::default());
            if workers == Workers::Pool && count > 1 {
                work.par_chunks_mut(TILE_SLOT_BYTES)
                    .zip(self.wave.par_iter())
                    .zip(self.encoded.par_iter_mut())
                    .try_for_each(|((slot, rect), out)| encode_tile(&previous, *rect, slot, out))?;
            } else {
                work.chunks_mut(TILE_SLOT_BYTES)
                    .zip(self.wave.iter())
                    .zip(self.encoded.iter_mut())
                    .try_for_each(|((slot, rect), out)| encode_tile(&previous, *rect, slot, out))?;
            }
            for ((slot, rect), out) in work
                .chunks_exact(TILE_SLOT_BYTES)
                .zip(self.wave.iter())
                .zip(self.encoded.iter())
            {
                let data = if out.encoding == ENCODING_RAW {
                    &slot[..out.len]
                } else {
                    &slot[RAW_TILE_BYTES..RAW_TILE_BYTES + out.len]
                };
                emit(*rect, out.encoding, data)?;
            }
        }
    }
}

/// What every row task of `look` shares.
struct Band<'a> {
    look: &'a Look<'a>,
    size: FrameSize,
    row_bytes: usize,
}

impl Band<'_> {
    /// One row of tiles: `pixels` are its rows of the previous picture.
    fn look(&self, row: u32, pixels: &mut [u8], marks: &mut [bool], changed: &mut [bool]) {
        let full = self.look.full;
        for (column, (mark, changed)) in marks.iter_mut().zip(changed.iter_mut()).enumerate() {
            *changed = false;
            let rect = tile_rect(column as u32, row, self.size);
            if !full && !visible(rect, self.look.visible) {
                continue;
            }
            // Untouched since it was last looked at: unchanged (P9).
            if !full && !*mark {
                continue;
            }
            *mark = false;
            let (start, end) = (rect.x as usize * 4, (rect.x + rect.width) as usize * 4);
            let lines = (0..rect.height).map(|dy| dy as usize * self.row_bytes);
            if !full
                && lines.clone().zip(rect.y..).all(|(at, y)| {
                    same_colour(
                        &pixels[at + start..at + end],
                        &self.look.view.row(y)[start..end],
                    )
                })
            {
                continue;
            }
            *changed = true;
            for (at, y) in lines.zip(rect.y..) {
                let target = &mut pixels[at + start..at + end];
                target.copy_from_slice(&self.look.view.row(y)[start..end]);
                for pixel in target.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
        }
    }
}

fn word(bytes: &[u8]) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(bytes);
    u64::from_le_bytes(word)
}

/// Whether two BGRA lines have the same colours (alpha ignored).
fn same_colour(old: &[u8], new: &[u8]) -> bool {
    let (old_words, new_words) = (old.chunks_exact(8), new.chunks_exact(8));
    let tail = old_words
        .remainder()
        .iter()
        .zip(new_words.remainder())
        .take(3)
        .all(|(a, b)| a == b);
    tail && old_words
        .zip(new_words)
        .all(|(a, b)| (word(a) ^ word(b)) & !ALPHA_MASK == 0)
}

/// Gathers the tile `rect` of `previous` into the slot and encodes it.
fn encode_tile(
    previous: &BgraView<'_>,
    rect: Rect,
    slot: &mut [u8],
    out: &mut Encoded,
) -> io::Result<()> {
    let (tile, compressed) = slot.split_at_mut(RAW_TILE_BYTES);
    let line = rect.width as usize * 4;
    let length = line * rect.height as usize;
    let start = rect.x as usize * 4;
    let tile = &mut tile[..length];
    for (target, y) in tile.chunks_exact_mut(line).zip(rect.y..) {
        target.copy_from_slice(&previous.row(y)[start..start + line]);
    }
    for pixel in tile.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    *out = if let Some(colour) = solid_colour(tile) {
        compressed[..4].copy_from_slice(&colour);
        Encoded {
            encoding: ENCODING_SOLID,
            len: 4,
        }
    } else {
        let written = compress_into(tile, compressed)
            .map_err(|_| invalid("admitted tile compression failed"))?;
        if written < length {
            Encoded {
                encoding: ENCODING_LZ4,
                len: written,
            }
        } else {
            Encoded {
                encoding: ENCODING_RAW,
                len: length,
            }
        }
    };
    Ok(())
}

#[cfg(test)]
#[path = "tiles_parallel_tests.rs"]
mod tests;
