//! Which 64 × 64 tiles of the lossless tile codec may have changed, from the
//! rectangles the GPU's desktop composition names (plan 2026-11-17 P9, the
//! fast path of „Verlustfrei“). Desktop Duplication hands every picture its
//! dirty and move rectangles (`os/windows/dxgi_regions.rs`, `BgraImage::
//! regions`); a tile no rectangle touches did not change, so the encoder
//! neither hashes nor compares it. Today's tile encoder hashes and compares
//! every tile of every picture (24.9 ms at 2880 × 1800 on 2026-11-17); with
//! the mask its work follows the changed area: typing touches 1–4 of the
//! 1305 tiles of that screen.
//!
//! Correctness rules:
//! - A picture without rectangles (unknown: rotated output, pointer moved,
//!   a dropped predecessor, another capture route) marks every tile.
//! - A tile stays marked until the encoder has looked at it (`clear`): a tile
//!   it skipped as invisible or for want of record room is looked at later.
//! - Every picture the pipeline takes must be noted, also one it drops
//!   before encoding; otherwise the rectangles of the dropped picture are
//!   lost. A caller that cannot guarantee this calls `mark_all`.
//! - Move rectangles mark their destination (the source keeps its pixels
//!   unless a dirty rectangle says otherwise).
use super::frame_regions::FrameRegions;
use super::pixel_convert::{FrameSize, Rect};
use super::tiles_codec::TILE_SIDE;

/// The tiles of one picture size that may differ from what the viewer has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirtyTiles {
    size: FrameSize,
    columns: u32,
    rows: u32,
    marked: Vec<bool>,
}

impl DirtyTiles {
    /// Every tile of `size` marked (the first picture is a key frame).
    pub fn new(size: FrameSize) -> Self {
        let columns = size.width.div_ceil(TILE_SIDE);
        let rows = size.height.div_ceil(TILE_SIDE);
        Self {
            size,
            columns,
            rows,
            marked: vec![true; columns as usize * rows as usize],
        }
    }

    pub fn size(&self) -> FrameSize {
        self.size
    }

    /// Marks every tile (unknown change, a key frame, a new reference).
    pub fn mark_all(&mut self) {
        self.marked.iter_mut().for_each(|tile| *tile = true);
    }

    /// Notes one taken picture of this size with its `regions` (`None`:
    /// unknown, every tile). Regions that do not fit the size mark every
    /// tile as well.
    pub fn note(&mut self, size: FrameSize, regions: Option<&FrameRegions>) {
        let Some(regions) = regions.filter(|r| size == self.size && r.validate(size).is_ok())
        else {
            self.mark_all();
            return;
        };
        for rect in &regions.dirty {
            self.mark_rect(*rect);
        }
        for moved in &regions.moves {
            self.mark_rect(moved.destination);
        }
    }

    /// Marks the tiles `rect` touches, e.g. the old and new place of a
    /// pointer drawn into the picture; parts outside the picture are ignored.
    pub fn mark_rect(&mut self, rect: Rect) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let right = u64::from(rect.x) + u64::from(rect.width);
        let bottom = u64::from(rect.y) + u64::from(rect.height);
        let first_column = rect.x / TILE_SIDE;
        let first_row = rect.y / TILE_SIDE;
        let end_column = tile_end(right, self.columns);
        let end_row = tile_end(bottom, self.rows);
        for row in first_row..end_row {
            for column in first_column..end_column {
                self.marked[(row * self.columns + column) as usize] = true;
            }
        }
    }

    /// Whether the tile at `column`, `row` may have changed; outside the
    /// grid: `true` (the encoder then looks, as without a mask).
    pub fn marked(&self, column: u32, row: u32) -> bool {
        if column >= self.columns || row >= self.rows {
            return true;
        }
        self.marked[(row * self.columns + column) as usize]
    }

    /// The encoder looked at the tile (sent it or found it equal).
    pub fn clear(&mut self, column: u32, row: u32) {
        if column < self.columns && row < self.rows {
            self.marked[(row * self.columns + column) as usize] = false;
        }
    }

    /// The marks row by row (`columns` per row), so that an encoder can
    /// look at the rows of tiles on several threads (`tiles_parallel.rs`).
    pub fn marks_mut(&mut self) -> &mut [bool] {
        &mut self.marked
    }

    /// How many tiles are marked (statistics, tests).
    pub fn count(&self) -> usize {
        self.marked.iter().filter(|tile| **tile).count()
    }
}

/// The first tile index past the pixel edge `end`, at most `limit`.
fn tile_end(end: u64, limit: u32) -> u32 {
    let tiles = end.div_ceil(u64::from(TILE_SIDE));
    u32::try_from(tiles).map_or(limit, |tiles| tiles.min(limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::frame_regions::MoveRect;

    const SIZE: FrameSize = FrameSize {
        width: 2880,
        height: 1800,
    };

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn clean() -> DirtyTiles {
        let mut tiles = DirtyTiles::new(SIZE);
        for row in 0..SIZE.height.div_ceil(64) {
            for column in 0..SIZE.width.div_ceil(64) {
                tiles.clear(column, row);
            }
        }
        tiles
    }

    #[test]
    fn tiles_dirty_task_new_and_unknown_mark_every_tile() {
        let tiles = DirtyTiles::new(SIZE);
        assert_eq!(tiles.count(), 45 * 29);
        let mut tiles = clean();
        assert_eq!(tiles.count(), 0);
        tiles.note(SIZE, None);
        assert_eq!(tiles.count(), 45 * 29);
        // Regions of another size, or ones that do not fit, mark everything.
        let mut tiles = clean();
        let other = FrameRegions {
            moves: Vec::new(),
            dirty: vec![rect(0, 0, 10, 10)],
        };
        tiles.note(FrameSize::new(1920, 1080), Some(&other));
        assert_eq!(tiles.count(), 45 * 29);
        let mut tiles = clean();
        let outside = FrameRegions {
            moves: Vec::new(),
            dirty: vec![rect(2870, 0, 20, 10)],
        };
        tiles.note(SIZE, Some(&outside));
        assert_eq!(tiles.count(), 45 * 29);
    }

    #[test]
    fn tiles_dirty_task_typing_marks_only_touched_tiles() {
        let mut tiles = clean();
        // One character across a tile edge: two tiles.
        let typed = FrameRegions {
            moves: Vec::new(),
            dirty: vec![rect(60, 10, 8, 16)],
        };
        tiles.note(SIZE, Some(&typed));
        assert_eq!(tiles.count(), 2);
        assert!(tiles.marked(0, 0) && tiles.marked(1, 0));
        assert!(!tiles.marked(2, 0) && !tiles.marked(0, 1));
        // An empty region list (no change) marks nothing more.
        tiles.note(SIZE, Some(&FrameRegions::default()));
        assert_eq!(tiles.count(), 2);
        // The last partial row and column are reachable.
        // Row 28 starts at 1792 (64-pixel tiles).
        tiles.mark_rect(rect(2870, 1795, 10, 5));
        assert!(tiles.marked(44, 28));
        assert_eq!(tiles.count(), 3);
    }

    #[test]
    fn tiles_dirty_task_marks_stay_until_cleared_and_moves_mark_destination() {
        let mut tiles = clean();
        let moved = FrameRegions {
            moves: vec![MoveRect {
                source_x: 0,
                source_y: 0,
                destination: rect(128, 128, 64, 64),
            }],
            dirty: Vec::new(),
        };
        tiles.note(SIZE, Some(&moved));
        assert!(tiles.marked(2, 2));
        assert!(!tiles.marked(0, 0));
        // A later picture without change keeps the unlooked tile marked.
        tiles.note(SIZE, Some(&FrameRegions::default()));
        assert!(tiles.marked(2, 2));
        tiles.clear(2, 2);
        assert_eq!(tiles.count(), 0);
        // Outside the grid the encoder looks; clearing there is harmless.
        assert!(tiles.marked(45, 0));
        tiles.clear(45, 29);
        tiles.mark_rect(rect(0, 0, 0, 5));
        assert_eq!(tiles.count(), 0);
        tiles.mark_all();
        assert_eq!(tiles.count(), 45 * 29);
    }
}
