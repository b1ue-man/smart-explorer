//! The pointer of a Desktop Duplication capture (plan 2026-11-14 bildkette,
//! B-13). The duplication delivers the pointer beside the picture: position
//! and visibility in `DXGI_OUTDUPL_FRAME_INFO.PointerPosition` (when
//! `LastMouseUpdateTime` is set), the shape through `GetFramePointerShape`
//! (when `PointerShapeBufferSize` is set). Both go to the pointer overlay of
//! the host process (`core::cursor_overlay`, P6 frames); while the viewer
//! does not draw the overlay, the host blends the pointer into the picture,
//! so "Ansehen" shows it too.
//!
//! Shapes: a color pointer is 32-bit BGRA with straight alpha; a masked color
//! pointer replaces the screen where its alpha byte is 0 and XORs it where
//! the byte is 0xFF; a monochrome pointer has an AND and an XOR mask of one
//! bit per pixel, one above the other (`Height` is twice the pointer's). The
//! decoder renders each shape onto black and onto white as the display would
//! and hands both to `cursor_mask::from_two_backgrounds`, so inverting pixels
//! become black with a white edge exactly as with the Graphics Capture
//! sampler. [A: layouts and XOR rules as in Microsoft's documentation of
//! `DXGI_OUTDUPL_POINTER_SHAPE_TYPE` and its DesktopDuplication sample;
//! neither was fetched in this session.]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::Win32::Graphics::Dxgi::{
    IDXGIOutputDuplication, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO,
};

use super::super::capture_slots::{CaptureBudget, CaptureKind};
use crate::screen::core::capture_image::AdmittedRegions;
use crate::screen::core::capture_route::pointer_patch;
use crate::screen::core::cursor_overlay::{from_two_backgrounds, worker, CursorShape};
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize, Rect};

/// `DXGI_OUTDUPL_POINTER_SHAPE_TYPE` values (windows 0.58 `Dxgi/mod.rs`).
pub(in crate::screen::os) const SHAPE_MONOCHROME: u32 = 1;
pub(in crate::screen::os) const SHAPE_COLOR: u32 = 2;
pub(in crate::screen::os) const SHAPE_MASKED_COLOR: u32 = 4;
/// Largest pointer side read; the same bound as the Graphics Capture sampler
/// (`cursor_overlay.rs` `MAX_SIDE`) and the overlay (`MAX_SOURCE_SIDE`).
const MAX_SIDE: u32 = 512;
/// Largest shape buffer read: a color pointer of `MAX_SIDE` squared.
const MAX_SHAPE_BYTES: u32 = MAX_SIDE * MAX_SIDE * 4;

/// A decoded pointer: RGBA with premultiplied colors, hot spot in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::screen::os) struct PointerImage {
    pub width: u32,
    pub height: u32,
    pub hot_x: i32,
    pub hot_y: i32,
    pub rgba: Vec<u8>,
}

fn bit(bytes: &[u8], pitch: usize, row: u32, column: u32) -> Option<bool> {
    let byte = bytes.get(row as usize * pitch + column as usize / 8)?;
    Some(byte & (0x80 >> (column % 8)) != 0)
}

fn bgra(bytes: &[u8], pitch: usize, row: u32, column: u32) -> Option<[u8; 4]> {
    let at = row as usize * pitch + column as usize * 4;
    let pixel = bytes.get(at..at + 4)?;
    Some([pixel[0], pixel[1], pixel[2], pixel[3]])
}

/// The pointer of a `GetFramePointerShape` buffer; `None` for an unknown
/// type, an empty or oversized shape or a buffer shorter than its pitch says.
pub(in crate::screen::os) fn decode(
    kind: u32,
    width: u32,
    height: u32,
    pitch: u32,
    hot: (i32, i32),
    bytes: &[u8],
) -> Option<PointerImage> {
    let rows = if kind == SHAPE_MONOCHROME {
        height / 2
    } else {
        height
    };
    if width == 0 || rows == 0 || width > MAX_SIDE || rows > MAX_SIDE {
        return None;
    }
    let pitch = pitch as usize;
    let pixels = width as usize * rows as usize * 4;
    let (mut black, mut white) = (Vec::with_capacity(pixels), Vec::with_capacity(pixels));
    for y in 0..rows {
        for x in 0..width {
            let (dark, light) = match kind {
                SHAPE_MONOCHROME => {
                    // Screen = (screen AND and) XOR xor.
                    let and: u8 = if bit(bytes, pitch, y, x)? { 255 } else { 0 };
                    let xor: u8 = if bit(bytes, pitch, y + rows, x)? {
                        255
                    } else {
                        0
                    };
                    let light = and ^ xor;
                    ([xor, xor, xor, 255], [light, light, light, 255])
                }
                SHAPE_COLOR => {
                    let [b, g, r, a] = bgra(bytes, pitch, y, x)?;
                    let over = |c: u8| (u16::from(c) * u16::from(a) + 127) / 255;
                    let dark = [over(b), over(g), over(r)].map(|v| v as u8);
                    let light = dark.map(|v| (u16::from(v) + 255 - u16::from(a)) as u8);
                    (
                        [dark[0], dark[1], dark[2], 255],
                        [light[0], light[1], light[2], 255],
                    )
                }
                SHAPE_MASKED_COLOR => {
                    let [b, g, r, mask] = bgra(bytes, pitch, y, x)?;
                    let light = if mask == 0 {
                        [b, g, r, 255]
                    } else {
                        [255 - b, 255 - g, 255 - r, 255]
                    };
                    ([b, g, r, 255], light)
                }
                _ => return None,
            };
            black.extend_from_slice(&dark);
            white.extend_from_slice(&light);
        }
    }
    Some(PointerImage {
        width,
        height: rows,
        hot_x: hot.0,
        hot_y: hot.1,
        rgba: from_two_backgrounds(&black, &white, width as usize),
    })
}

/// The unrotated pixel (`x`, `y`) of a `native` texture as the display shows
/// it (the mapping of `pixel_convert::rotate_bgra`).
pub(in crate::screen::os) fn displayed(
    rotation: BgraRotation,
    native: FrameSize,
    x: u32,
    y: u32,
) -> (u32, u32) {
    let (width, height) = (native.width, native.height);
    match rotation {
        BgraRotation::Identity => (x, y),
        BgraRotation::Clockwise90 => (height - 1 - y, x),
        BgraRotation::Clockwise180 => (width - 1 - x, height - 1 - y),
        BgraRotation::Clockwise270 => (y, width - 1 - x),
    }
}

/// Blends `pointer` with its top left at (`left`, `top`) of the displayed
/// picture into `pixels`, the tight unrotated BGRA rows of a `delivered`
/// image read from a `native` texture (scaled pictures sample the pointer at
/// each target pixel's center).
pub(in crate::screen::os) fn blend(
    pixels: &mut [u8],
    delivered: FrameSize,
    native: FrameSize,
    rotation: BgraRotation,
    pointer: &PointerImage,
    left: i32,
    top: i32,
) {
    let (dw, dh, nw, nh) = (
        u64::from(delivered.width),
        u64::from(delivered.height),
        u64::from(native.width),
        u64::from(native.height),
    );
    if dw == 0 || dh == 0 || nw == 0 || nh == 0 || pixels.len() as u64 != dw * dh * 4 {
        return;
    }
    for y in 0..delivered.height {
        let ny = ((2 * u64::from(y) + 1) * nh / (2 * dh)) as u32;
        for x in 0..delivered.width {
            let nx = ((2 * u64::from(x) + 1) * nw / (2 * dw)) as u32;
            let (sx, sy) = displayed(rotation, native, nx, ny);
            let (px, py) = (
                i64::from(sx) - i64::from(left),
                i64::from(sy) - i64::from(top),
            );
            if px < 0 || py < 0 || px >= i64::from(pointer.width) || py >= i64::from(pointer.height)
            {
                continue;
            }
            let at = (py as usize * pointer.width as usize + px as usize) * 4;
            let source = &pointer.rgba[at..at + 4];
            let alpha = u16::from(source[3]);
            if alpha == 0 {
                continue;
            }
            let target = (y as usize * delivered.width as usize + x as usize) * 4;
            let under = &mut pixels[target..target + 4];
            // Premultiplied over: source + under * (1 - alpha); RGBA onto BGRA.
            for (channel, value) in [(0usize, source[2]), (1, source[1]), (2, source[0])] {
                let kept = (u16::from(under[channel]) * (255 - alpha) + 127) / 255;
                under[channel] = (u16::from(value) + kept).min(255) as u8;
            }
            under[3] = 255;
        }
    }
}

/// The pointer state of one duplication.
pub(super) struct DxgiPointer {
    /// Set by the overlay's switch; the capture then starts its duplication
    /// anew, so a picture in the new state (with or without pointer) follows.
    switched: Arc<AtomicBool>,
    shape: Option<PointerImage>,
    serial: u64,
    /// Top left of the shape on the output, displayed pixels.
    position: Option<(i32, i32)>,
    visible: bool,
    /// P9: the area the pointer covered before the last change (left, top,
    /// width, height), for the regions of a pointer-only change.
    previous: Option<(i32, i32, u32, u32)>,
}

impl DxgiPointer {
    /// Hands the overlay's switch to this duplication.
    pub(super) fn attach() -> Self {
        let switched = Arc::new(AtomicBool::new(false));
        super::super::cursor_overlay::attach_duplication(switched.clone());
        Self {
            switched,
            shape: None,
            serial: 0,
            position: None,
            visible: false,
            previous: None,
        }
    }

    /// The area the pointer covers in a blended picture now.
    fn covered(&self) -> Option<(i32, i32, u32, u32)> {
        let (left, top) = self.position.filter(|_| self.visible)?;
        let shape = self.shape.as_ref()?;
        Some((left, top, shape.width, shape.height))
    }

    pub(super) fn take_switched(&self) -> bool {
        self.switched.swap(false, Ordering::AcqRel)
    }

    /// Whether pictures carry the pointer (no overlay at the viewer).
    pub(super) fn blends(&self) -> bool {
        !worker().separated()
    }

    /// Reads this frame's pointer (before `ReleaseFrame`) and publishes it to
    /// the overlay; true when a blended picture must change.
    pub(super) fn update(
        &mut self,
        duplication: &IDXGIOutputDuplication,
        info: &DXGI_OUTDUPL_FRAME_INFO,
        area: FrameSize,
    ) -> bool {
        let overlay = worker();
        let mut changed = false;
        let before = self.covered();
        if info.PointerShapeBufferSize > 0 {
            if let Some(image) = read_shape(duplication, info.PointerShapeBufferSize) {
                self.serial = self.serial.wrapping_add(1);
                let shape = CursorShape::sampled(
                    overlay.next_shape_id(),
                    image.width,
                    image.height,
                    image.hot_x,
                    image.hot_y,
                    |x, y| {
                        let at = ((y * image.width + x) * 4) as usize;
                        let p = &image.rgba[at..at + 4];
                        [p[0], p[1], p[2], p[3]]
                    },
                );
                overlay.publish_shape(self.serial, shape);
                self.shape = Some(image);
                changed = true;
            }
        }
        if info.LastMouseUpdateTime != 0 {
            let position = info.PointerPosition.Position;
            let visible = info.PointerPosition.Visible.as_bool();
            let at = Some((position.x, position.y));
            changed |= at != self.position || visible != self.visible;
            self.position = at;
            self.visible = visible;
        }
        if let (Some((left, top)), Some(shape)) = (self.position, &self.shape) {
            let (x, y) = (left + shape.hot_x, top + shape.hot_y);
            let inside = self.visible
                && x >= 0
                && y >= 0
                && (x as u32) < area.width
                && (y as u32) < area.height;
            overlay.publish_position(x, y, area, inside);
        }
        if changed {
            self.previous = before;
        }
        changed && self.blends()
    }

    /// The pointer a picture on the GPU route carries (plan 2026-11-17 P9,
    /// `capture_pointer_gpu.rs`): the same condition as `blend_into`, in the
    /// mode `blends` the capture took once for the picture (round 9 H4).
    pub(super) fn on_picture(&self, blends: bool) -> Option<(&PointerImage, i32, i32)> {
        if !blends || !self.visible {
            return None;
        }
        let (left, top) = self.position?;
        Some((self.shape.as_ref()?, left, top))
    }

    /// Blends the pointer into a picture before it is frozen.
    pub(super) fn blend_into(
        &self,
        pixels: &mut [u8],
        delivered: FrameSize,
        native: FrameSize,
        rotation: BgraRotation,
        blends: bool,
    ) {
        if !blends || !self.visible {
            return;
        }
        if let (Some((left, top)), Some(shape)) = (self.position, &self.shape) {
            blend(pixels, delivered, native, rotation, shape, left, top);
        }
    }
}

/// P9: the regions of a picture whose blended pointer moved: the dirty and
/// move rectangles of the duplication plus the pointer's old and new area,
/// or only those two areas when the desktop did not change (`only`). `None`
/// (the whole picture changed, as before) for a rotated or scaled capture,
/// without a pointer area, or when the rectangles exceed their budget.
pub(super) fn with_moved(
    pointer: Option<&DxgiPointer>,
    regions: Option<AdmittedRegions>,
    only: bool,
    budget: &CaptureBudget,
) -> Option<AdmittedRegions> {
    let geometry = budget.geometry();
    let plain = geometry.kind == CaptureKind::Dxgi(BgraRotation::Identity)
        && geometry.delivered == geometry.native;
    let pointer = pointer.filter(|_| plain && (regions.is_some() || only))?;
    let size = geometry.native;
    let areas: Vec<Rect> = [pointer.previous, pointer.covered()]
        .into_iter()
        .flatten()
        .filter_map(|(left, top, width, height)| pointer_patch(size, left, top, width, height))
        .collect();
    if areas.is_empty() {
        return None;
    }
    let (moves, dirty) = regions.map_or((Vec::new(), Vec::new()), |regions| {
        let all = regions.regions();
        (all.moves.clone(), all.dirty.clone())
    });
    let count = dirty.len() + areas.len();
    let mut out =
        AdmittedRegions::new(size, budget.grant().ok()?, moves.len() as u64, count as u64).ok()?;
    for moved in moves {
        out.push_move(moved).ok()?;
    }
    for area in dirty.into_iter().chain(areas) {
        out.push_dirty(area).ok()?;
    }
    Some(out)
}

fn read_shape(duplication: &IDXGIOutputDuplication, size: u32) -> Option<PointerImage> {
    if size > MAX_SHAPE_BYTES {
        return None;
    }
    // Pointer bytes are bounded above and not frame pixels: like the
    // Graphics Capture sampler they are not part of the capture grant.
    let mut buffer = vec![0u8; size as usize];
    let mut required = 0u32;
    let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
    // SAFETY: the buffer holds `size` bytes, the out pointers address locals.
    unsafe {
        duplication.GetFramePointerShape(size, buffer.as_mut_ptr().cast(), &mut required, &mut info)
    }
    .ok()?;
    let used = buffer.get(..required.min(size) as usize)?;
    decode(
        info.Type,
        info.Width,
        info.Height,
        info.Pitch,
        (info.HotSpot.x, info.HotSpot.y),
        used,
    )
}
