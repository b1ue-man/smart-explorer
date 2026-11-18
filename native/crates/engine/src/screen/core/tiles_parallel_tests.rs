//! The parallel tile encoder gives the tiles of the sequential one (P9).
use super::*;
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureOwner, CapturePlan, SourceKey,
};
use crate::screen::core::frame_regions::FrameRegions;
use crate::screen::core::host_codec_adapters::{
    HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan,
};
use crate::screen::core::tiles_codec::tile_hash;
use lz4_flex::block::get_maximum_output_size;

type Tile = (Rect, u8, Vec<u8>);

/// xorshift64: reproducible pictures without a dependency.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Fills `area` of a tight picture: one colour, a gradient or noise, each
/// with arbitrary alpha bytes.
fn paint(picture: &mut [u8], size: FrameSize, area: Rect, kind: u64, noise: &mut Noise) {
    let colour = [noise.byte(), noise.byte(), noise.byte()];
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let at = (y * size.width + x) as usize * 4;
            let pixel = match kind % 3 {
                0 => colour,
                1 => [x as u8, y as u8, (x / 3) as u8],
                _ => [noise.byte(), noise.byte(), noise.byte()],
            };
            picture[at..at + 3].copy_from_slice(&pixel);
            picture[at + 3] = noise.byte();
        }
    }
}

fn picture(size: FrameSize, noise: &mut Noise) -> Vec<u8> {
    let mut picture = vec![0u8; size.width as usize * size.height as usize * 4];
    for row in 0..size.height.div_ceil(TILE_SIDE) {
        for column in 0..size.width.div_ceil(TILE_SIDE) {
            let kind = noise.next();
            paint(
                &mut picture,
                size,
                tile_rect(column, row, size),
                kind,
                noise,
            );
        }
    }
    picture
}

fn view(size: FrameSize, picture: &[u8]) -> BgraView<'_> {
    BgraView::new(size, size.width as usize * 4, picture).unwrap()
}

/// The tile loop of `AdmittedTilesEncoder` before P9 tiles-parallel: hash,
/// compare, gather, compress, one tile after the other.
struct Reference {
    size: FrameSize,
    previous: Vec<u8>,
    hashes: Vec<u64>,
    dirty: DirtyTiles,
}

impl Reference {
    fn new(size: FrameSize) -> Self {
        let tiles = size.width.div_ceil(TILE_SIDE) * size.height.div_ceil(TILE_SIDE);
        Self {
            size,
            previous: vec![0; size.width as usize * size.height as usize * 4],
            hashes: vec![0; tiles as usize],
            dirty: DirtyTiles::new(size),
        }
    }

    fn frame(&mut self, look: &Look<'_>) -> Vec<Tile> {
        let (size, frame, full) = (self.size, look.view, look.full);
        let row_bytes = size.width as usize * 4;
        let columns = size.width.div_ceil(TILE_SIDE);
        let mut out = Vec::new();
        for row in 0..size.height.div_ceil(TILE_SIDE) {
            for column in 0..columns {
                let rect = tile_rect(column, row, size);
                if !full && !visible(rect, look.visible) {
                    continue;
                }
                if !full && !self.dirty.marked(column, row) {
                    continue;
                }
                self.dirty.clear(column, row);
                let hash = tile_hash(&frame, rect);
                let slot = (row * columns + column) as usize;
                let (start, end) = (rect.x as usize * 4, (rect.x + rect.width) as usize * 4);
                let exact = self.hashes[slot] == hash
                    && (rect.y..rect.y + rect.height).all(|y| {
                        let at = y as usize * row_bytes;
                        self.previous[at + start..at + end]
                            .chunks_exact(4)
                            .zip(frame.row(y)[start..end].chunks_exact(4))
                            .all(|(a, b)| a[..3] == b[..3])
                    });
                if !full && exact {
                    continue;
                }
                let mut tile = Vec::new();
                for y in rect.y..rect.y + rect.height {
                    tile.extend_from_slice(&frame.row(y)[start..end]);
                }
                for pixel in tile.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
                let (encoding, bytes) = if let Some(colour) = solid_colour(&tile) {
                    (ENCODING_SOLID, colour.to_vec())
                } else {
                    let mut packed = vec![0u8; get_maximum_output_size(tile.len())];
                    let written = compress_into(&tile, &mut packed).unwrap();
                    if written < tile.len() {
                        (ENCODING_LZ4, packed[..written].to_vec())
                    } else {
                        (ENCODING_RAW, tile.clone())
                    }
                };
                out.push((rect, encoding, bytes));
                self.hashes[slot] = hash;
                for (index, y) in (rect.y..rect.y + rect.height).enumerate() {
                    let at = y as usize * row_bytes;
                    let line = rect.width as usize * 4;
                    self.previous[at + start..at + end]
                        .copy_from_slice(&tile[index * line..(index + 1) * line]);
                }
            }
        }
        out
    }
}

/// The parallel encoder with its own previous picture, marks and slots.
struct Engine {
    size: FrameSize,
    previous: Vec<u8>,
    dirty: DirtyTiles,
    work: TileWork,
    scratch: Vec<u8>,
    workers: Workers,
}

impl Engine {
    fn new(size: FrameSize, slots: usize, workers: Workers) -> Self {
        Self {
            size,
            previous: vec![0; size.width as usize * size.height as usize * 4],
            dirty: DirtyTiles::new(size),
            work: TileWork::new(size),
            scratch: vec![0; slots * TILE_SLOT_BYTES],
            workers,
        }
    }

    fn frame(&mut self, look: &Look<'_>) -> Vec<Tile> {
        let changed = self
            .work
            .look(&mut self.previous, look, &mut self.dirty, self.workers)
            .unwrap();
        let mut out = Vec::new();
        let previous = view(self.size, &self.previous);
        self.work
            .encode(
                previous,
                &mut self.scratch,
                self.workers,
                |rect, encoding, bytes| {
                    out.push((rect, encoding, bytes.to_vec()));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(out.len(), changed);
        out
    }
}

/// What the capture says changed before a frame.
enum Marks<'a> {
    All,
    Regions(&'a FrameRegions),
}

fn mark(dirty: &mut DirtyTiles, marks: &Marks<'_>) {
    match marks {
        Marks::All => dirty.mark_all(),
        Marks::Regions(regions) => dirty.note(dirty.size(), Some(regions)),
    }
}

/// Runs one scenario through the reference and every engine variant and
/// checks the tiles, their order and the previous pictures.
fn scenario(size: FrameSize, seed: u64) {
    let mut noise = Noise(seed);
    let mut reference = Reference::new(size);
    let mut engines = [
        Engine::new(size, TILE_WAVE, Workers::Pool),
        Engine::new(size, TILE_WAVE, Workers::Caller),
        Engine::new(size, 1, Workers::Pool),
        Engine::new(size, 3, Workers::Pool),
    ];
    let mut frame = picture(size, &mut noise);
    let (last_column, last_row) = (
        size.width.div_ceil(TILE_SIDE) - 1,
        size.height.div_ceil(TILE_SIDE) - 1,
    );
    let corner = tile_rect(last_column, last_row, size);
    let half = vec![VisibleRect {
        x: 0,
        y: 0,
        width: size.width / 2,
        height: size.height,
    }];
    let typed = FrameRegions {
        moves: Vec::new(),
        dirty: vec![rect(60, 10, 8, 16), rect(64, 64, 64, 64)],
    };
    let unchanged = FrameRegions::default();
    let corner_only = FrameRegions {
        moves: Vec::new(),
        dirty: vec![corner],
    };
    let mut sent = 0;
    for step in 0..6u32 {
        let (full, marks, visible): (bool, Marks<'_>, Option<&[VisibleRect]>) = match step {
            0 => (true, Marks::All, None),
            1 => {
                // Typing in two tiles, alpha only in a third, an unmarked
                // change in the corner tile (not looked at).
                paint(&mut frame, size, rect(60, 10, 8, 16), 2, &mut noise);
                let alpha = (70 * size.width as usize + 70) * 4 + 3;
                frame[alpha] = frame[alpha].wrapping_add(1);
                paint(&mut frame, size, corner, 2, &mut noise);
                (false, Marks::Regions(&typed), None)
            }
            2 => {
                // Everything changes, only the left half is visible.
                frame = picture(size, &mut noise);
                (false, Marks::All, Some(half.as_slice()))
            }
            3 => {
                // Nothing new: the right half is still marked and now visible.
                (false, Marks::Regions(&unchanged), None)
            }
            4 => {
                // The corner, marked now.
                paint(&mut frame, size, corner, 1, &mut noise);
                (false, Marks::Regions(&corner_only), None)
            }
            _ => (true, Marks::All, Some(half.as_slice())),
        };
        let look = Look {
            view: view(size, &frame),
            full,
            visible,
        };
        mark(&mut reference.dirty, &marks);
        let expected = reference.frame(&look);
        for engine in &mut engines {
            mark(&mut engine.dirty, &marks);
            let tiles = engine.frame(&look);
            assert_eq!(tiles, expected, "step {step}, {:?}", engine.workers);
            assert_eq!(engine.previous, reference.previous, "step {step}");
            assert_eq!(engine.dirty, reference.dirty, "step {step}");
        }
        sent += expected.len();
        if step == 1 {
            // Two typed tiles; the alpha tile and the unmarked corner stay.
            assert_eq!(expected.len(), 2, "step 1");
        }
        if step == 3 {
            assert!(!expected.is_empty(), "the hidden half follows");
        }
    }
    assert!(sent > 0);
}

#[test]
fn tiles_parallel_task_records_equal_the_sequential_encoder() {
    // Odd edge tiles (13 × 8 pixels) and fewer tiles than a wave.
    scenario(FrameSize::new(333, 200), 0x9E37_79B9_7F4A_7C15);
    // Several waves of `TILE_WAVE` tiles per frame.
    scenario(FrameSize::new(1000, 600), 0x2545_F491_4F6C_DD1D);
}

#[test]
fn tiles_parallel_task_refuses_wrong_buffers() {
    let size = FrameSize::new(128, 64);
    let frame = picture(size, &mut Noise(7));
    let look = Look {
        view: view(size, &frame),
        full: true,
        visible: None,
    };
    let mut work = TileWork::new(size);
    let mut dirty = DirtyTiles::new(size);
    let mut short = vec![0u8; 16];
    assert!(work
        .look(&mut short, &look, &mut dirty, Workers::Pool)
        .is_err());
    let mut other = DirtyTiles::new(FrameSize::new(64, 64));
    let mut previous = vec![0u8; 128 * 64 * 4];
    assert!(work
        .look(&mut previous, &look, &mut other, Workers::Pool)
        .is_err());
    assert_eq!(
        work.look(&mut previous, &look, &mut dirty, Workers::Pool)
            .unwrap(),
        2
    );
    let mut none = vec![0u8; TILE_SLOT_BYTES - 1];
    let result = work.encode(
        view(size, &previous),
        &mut none,
        Workers::Pool,
        |_, _, _| Ok(()),
    );
    assert!(result.is_err());
}

#[test]
fn tiles_parallel_task_plan_admits_a_wave_of_slots() {
    let mut plan = CapturePlan::new(
        SourceKey::new("tiles", "wave").unwrap(),
        CaptureGeneration::new(1),
    );
    HostOutputPlan::new(
        HostContract::Legacy,
        FrameSize::new(1920, 1080),
        HostCodecPlan::Tiles,
    )
    .unwrap()
    .append_to(&mut plan, HostInputPixels::SeparateImage)
    .unwrap();
    // The live and the snapshot encoder each claim one wave of slots.
    let scratch = plan.owner_limits()[CaptureOwner::FilterScratch as usize];
    assert!(
        scratch >= 2 * (TILE_WAVE * TILE_SLOT_BYTES) as u64,
        "{scratch}"
    );
}
