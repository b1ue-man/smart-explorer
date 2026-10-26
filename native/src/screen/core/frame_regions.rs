//! Capture facts and deterministic pixel comparison; no platform discovery.
use std::io;
use super::pixel_convert::{BgraImage, FrameSize, Rect};
use super::wire::invalid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MoveRect { pub source_x: u32, pub source_y: u32, pub destination: Rect }
impl MoveRect {
    pub(crate) fn validate(self, size: FrameSize) -> io::Result<()> {
        let source = Rect { x: self.source_x, y: self.source_y, ..self.destination };
        if !source.fits(size) || !self.destination.fits(size) { return Err(invalid("screen move rectangle exceeds frame")); }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameRegions { pub moves: Vec<MoveRect>, pub dirty: Vec<Rect> }
impl FrameRegions {
    pub(crate) fn validate(&self, size: FrameSize) -> io::Result<()> {
        let max_rects = size.width.div_ceil(64) as usize * size.height.div_ceil(64) as usize;
        if self.moves.len().saturating_add(self.dirty.len()) > max_rects.saturating_mul(2) {
            return Err(invalid("too many capture rectangles"));
        }
        for moved in &self.moves { moved.validate(size)?; }
        if self.dirty.iter().any(|rect| !rect.fits(size)) { return Err(invalid("capture dirty rectangle exceeds frame")); }
        Ok(())
    }
}
pub(crate) fn grid(size: FrameSize) -> Vec<Rect> {
    (0..size.height).step_by(64).flat_map(|y| (0..size.width).step_by(64).map(move |x|
        Rect { x, y, width: 64.min(size.width - x), height: 64.min(size.height - y) })).collect()
}
pub(crate) fn same(previous: &BgraImage, current: &BgraImage, rect: Rect) -> bool {
    if previous.size() != current.size() || !rect.fits(current.size()) { return false; }
    let (start, end) = (rect.x as usize * 4, (rect.x + rect.width) as usize * 4);
    (rect.y..rect.y + rect.height).all(|y| previous.view().row(y)[start..end].chunks_exact(4)
        .zip(current.view().row(y)[start..end].chunks_exact(4)).all(|(a,b)| a[..3] == b[..3]))
}
pub(crate) fn copy_rect(source: &BgraImage, src: Rect, target: &mut BgraImage, dst: Rect) -> io::Result<()> {
    if !src.fits(source.size()) || !dst.fits(target.size()) || src.width != dst.width || src.height != dst.height {
        return Err(invalid("invalid screen rectangle copy"));
    }
    for row in 0..src.height {
        let from = src.x as usize * 4; let to = dst.x as usize * 4; let len = src.width as usize * 4;
        target.row_mut(dst.y + row)[to..to+len].copy_from_slice(&source.view().row(src.y + row)[from..from+len]);
    }
    Ok(())
}

pub(crate) fn intersects(a: Rect,b: Rect) -> bool {
    u64::from(a.x)<u64::from(b.x)+u64::from(b.width) && u64::from(b.x)<u64::from(a.x)+u64::from(a.width)
        && u64::from(a.y)<u64::from(b.y)+u64::from(b.height) && u64::from(b.y)<u64::from(a.y)+u64::from(a.height)
}
/// Rectangle coordinates are already projected to this encoder's input size.
pub(crate) fn visible(rect: Rect, regions: Option<&[super::visibility::VisibleRect]>) -> bool {
    regions.is_none_or(|regions|regions.iter().any(|r|intersects(rect,Rect {x:r.x,y:r.y,width:r.width,height:r.height})))
}
pub(crate) fn matches_move(previous: &BgraImage,current: &BgraImage,moved: MoveRect) -> bool {
    if previous.size()!=current.size() || moved.validate(current.size()).is_err() { return false; }
    let rect=moved.destination;
    (0..rect.height).all(|row| {
        let from=moved.source_x as usize*4; let to=rect.x as usize*4; let len=rect.width as usize*4;
        previous.view().row(moved.source_y+row)[from..from+len].chunks_exact(4)
            .zip(current.view().row(rect.y+row)[to..to+len].chunks_exact(4)).all(|(a,b)|a[..3]==b[..3])
    })
}
