//! Dirty and move rectangles of a Desktop Duplication frame (plan 2026-11-14
//! bildkette, BK2b): they are read before the picture, and an update without
//! any rectangle is "unchanged" without a readback or a full-picture
//! comparison. Microsoft's DesktopDuplication sample treats a zero
//! `TotalMetadataBufferSize` as no dirty and no move rectangle [A: sample and
//! documentation not fetched in this session]. Rectangles of an unscaled,
//! unrotated picture additionally become the image's regions.
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_image::AdmittedRegions;
use crate::screen::core::frame_regions::MoveRect;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize, Rect};
use crate::screen::os::capture_slots::{CaptureBudget, CaptureKind};
use std::io;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Dxgi::{IDXGIOutputDuplication, DXGI_OUTDUPL_MOVE_RECT};

/// What the rectangles of one frame say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum Verdict {
    /// No rectangle: the desktop picture did not change.
    Unchanged,
    Changed,
    /// The rectangles could not be read (rotated output without admitted
    /// metadata, refused memory, a driver error): the picture is read.
    Unknown,
}

/// The verdict of `metadata_bytes` and the read `counts` (moves, dirty) of
/// an `identity` (unrotated) output; rotated outputs have no admitted
/// metadata storage (`capture_slots.rs`).
pub(in crate::screen::os) fn verdict(
    identity: bool,
    metadata_bytes: u32,
    counts: Option<(usize, usize)>,
) -> Verdict {
    if metadata_bytes == 0 {
        return Verdict::Unchanged;
    }
    if !identity {
        return Verdict::Unknown;
    }
    match counts {
        Some((0, 0)) => Verdict::Unchanged,
        Some(_) => Verdict::Changed,
        None => Verdict::Unknown,
    }
}

/// The rectangles of the acquired frame: the verdict and, for an unscaled
/// unrotated picture of `size`, its regions.
pub(super) fn read(
    duplication: &IDXGIOutputDuplication,
    metadata_bytes: u32,
    budget: &CaptureBudget,
    size: FrameSize,
) -> (Verdict, Option<AdmittedRegions>) {
    let identity = budget.geometry().kind == CaptureKind::Dxgi(BgraRotation::Identity);
    if metadata_bytes == 0 || !identity {
        return (verdict(identity, metadata_bytes, None), None);
    }
    let native = budget.geometry().native;
    let count = u64::from(native.width.div_ceil(64)) * u64::from(native.height.div_ceil(64)) * 2;
    let fits = count
        .checked_mul(std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>() as u64)
        .is_some_and(|capacity| u64::from(metadata_bytes) <= capacity);
    let Some(grant) = budget.grant().ok().filter(|_| fits) else {
        return (Verdict::Unknown, None);
    };
    let Some((moved, used_moves, dirty, used_dirty)) = rectangles(duplication, count, grant) else {
        return (Verdict::Unknown, None);
    };
    let (Some(moves), Some(dirt)) = (moved.used(used_moves), dirty.used(used_dirty)) else {
        return (Verdict::Unknown, None);
    };
    let result = verdict(true, metadata_bytes, Some((moves.len(), dirt.len())));
    let regions = (result == Verdict::Changed && native == size)
        .then(|| regions(size, grant, moves, dirt))
        .flatten();
    (result, regions)
}

fn rectangles(
    duplication: &IDXGIOutputDuplication,
    count: u64,
    grant: &CaptureGrant,
) -> Option<(
    NativeRects<DXGI_OUTDUPL_MOVE_RECT>,
    u32,
    NativeRects<RECT>,
    u32,
)> {
    let mut moved = NativeRects::<DXGI_OUTDUPL_MOVE_RECT>::new(count, grant).ok()?;
    let mut dirty = NativeRects::<RECT>::new(count, grant).ok()?;
    let mut used_moves = 0;
    let mut used_dirty = 0;
    // SAFETY: admitted aligned typed buffers; all returned lengths are
    // validated before indexing or constructing immutable core metadata.
    unsafe {
        duplication
            .GetFrameMoveRects(moved.bytes, moved.values.as_mut_ptr(), &mut used_moves)
            .ok()?;
        duplication
            .GetFrameDirtyRects(dirty.bytes, dirty.values.as_mut_ptr(), &mut used_dirty)
            .ok()?;
    }
    // The admitted buffers stay the only copy of the rectangles.
    Some((moved, used_moves, dirty, used_dirty))
}

fn regions(
    size: FrameSize,
    grant: &CaptureGrant,
    moves: &[DXGI_OUTDUPL_MOVE_RECT],
    dirt: &[RECT],
) -> Option<AdmittedRegions> {
    let mut result =
        AdmittedRegions::new(size, grant, moves.len() as u64, dirt.len() as u64).ok()?;
    for moved in moves {
        result
            .push_move(MoveRect {
                source_x: u32::try_from(moved.SourcePoint.x).ok()?,
                source_y: u32::try_from(moved.SourcePoint.y).ok()?,
                destination: rect(moved.DestinationRect)?,
            })
            .ok()?;
    }
    for dirty in dirt {
        result.push_dirty(rect(*dirty)?).ok()?;
    }
    Some(result)
}

fn rect(r: RECT) -> Option<Rect> {
    Some(Rect {
        x: u32::try_from(r.left).ok()?,
        y: u32::try_from(r.top).ok()?,
        width: u32::try_from(r.right.checked_sub(r.left)?).ok()?,
        height: u32::try_from(r.bottom.checked_sub(r.top)?).ok()?,
    })
}

struct NativeRects<T> {
    values: Vec<T>,
    bytes: u32,
    _slice: CaptureSlice,
}
impl<T: Default + Clone> NativeRects<T> {
    fn new(count: u64, grant: &CaptureGrant) -> io::Result<Self> {
        let bytes = count
            .checked_mul(std::mem::size_of::<T>() as u64)
            .ok_or_else(|| io::Error::other("DXGI metadata overflow"))?;
        let length = usize::try_from(count)
            .map_err(|_| io::Error::other("DXGI metadata count exceeds address space"))?;
        let capacity = u32::try_from(bytes)
            .map_err(|_| io::Error::other("DXGI metadata exceeds API capacity"))?;
        let slice = grant.claim(CaptureOwner::Metadata, bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(length)
            .map_err(|_| io::Error::other("DXGI metadata allocation failed"))?;
        values.resize(length, T::default());
        Ok(Self {
            values,
            bytes: capacity,
            _slice: slice,
        })
    }
    fn used(&self, bytes: u32) -> Option<&[T]> {
        let length = bytes as usize;
        if bytes > self.bytes || length % std::mem::size_of::<T>() != 0 {
            return None;
        }
        self.values.get(..length / std::mem::size_of::<T>())
    }
}
