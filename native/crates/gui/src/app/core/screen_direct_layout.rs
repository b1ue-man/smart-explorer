//! Geometry of the Direct3D 11 picture of the desktop viewer (plan
//! 2026-11-17 P8), without Windows: the child window covers the picture
//! area in physical pixels of the window's client area; the picture lies in
//! it where `screen_viewport::image_rect` puts it (fit or 1:1 with pan, the
//! same rectangle the input mapping uses); the video processor gets the
//! part of the picture inside the child (source and destination
//! rectangles), black around it. Everything egui draws over the picture
//! stays visible: interactive popups and menus over the picture hide the
//! child for as long as they are open (they work in every case), the
//! statistics field, the full-screen button, tooltips and hints are cut out
//! of the child's region (`SetWindowRgn`), so egui's drawing shows there.
use eframe::egui::{Order, Rect};

use crate::screen::core::pixel_convert::FrameSize;

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// `rect` in points as pixels at `ppp` pixels per point, edges rounded.
    pub fn from_points(rect: Rect, ppp: f32) -> Self {
        let px = |value: f32| (value * ppp).round() as i32;
        Self::new(
            px(rect.min.x),
            px(rect.min.y),
            px(rect.max.x),
            px(rect.max.y),
        )
    }

    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub fn is_empty(self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub fn intersect(self, other: Self) -> Self {
        Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        )
    }

    /// Moved by `-origin` (into the coordinates of a child at `origin`).
    pub fn relative_to(self, origin: Self) -> Self {
        Self::new(
            self.left - origin.left,
            self.top - origin.top,
            self.right - origin.left,
            self.bottom - origin.top,
        )
    }
}

/// Where the child window and the picture are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // read by the Windows presenter
pub struct DirectGeometry {
    /// The child window in the parent's client area.
    pub child: PixelRect,
    /// The picture, relative to the child; may reach beyond it (1:1 with pan).
    pub dest: PixelRect,
    /// Rectangles egui draws over the picture, relative to the child.
    pub cutouts: Vec<PixelRect>,
}

/// The geometry of a picture area `area` with the picture at `dest` (both
/// in points) and the overlays `overlays` (points).
pub fn geometry(area: Rect, dest: Rect, ppp: f32, overlays: &[Rect]) -> DirectGeometry {
    let child = PixelRect::from_points(area, ppp);
    let bounds = PixelRect::new(0, 0, child.width(), child.height());
    DirectGeometry {
        child,
        dest: PixelRect::from_points(dest, ppp).relative_to(child),
        cutouts: overlays
            .iter()
            .map(|rect| {
                PixelRect::from_points(*rect, ppp)
                    .relative_to(child)
                    .intersect(bounds)
            })
            .filter(|rect| !rect.is_empty())
            .collect(),
    }
}

/// The part of a picture of `source` pixels drawn at `dest` (relative to a
/// child of `width` × `height`) that lies inside the child: the source
/// rectangle in picture pixels and the destination rectangle in the child.
/// `None` when nothing of the picture is inside.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows presenter's
pub fn blit_rects(
    width: i32,
    height: i32,
    dest: PixelRect,
    source: FrameSize,
) -> Option<(PixelRect, PixelRect)> {
    let visible = dest.intersect(PixelRect::new(0, 0, width, height));
    if visible.is_empty() || dest.is_empty() || source.width == 0 || source.height == 0 {
        return None;
    }
    let scale_x = f64::from(source.width) / f64::from(dest.width());
    let scale_y = f64::from(source.height) / f64::from(dest.height());
    let map = |value: i32, origin: i32, scale: f64, max: u32| {
        ((f64::from(value - origin) * scale).round() as i64).clamp(0, i64::from(max)) as i32
    };
    let src = PixelRect::new(
        map(visible.left, dest.left, scale_x, source.width),
        map(visible.top, dest.top, scale_y, source.height),
        map(visible.right, dest.left, scale_x, source.width),
        map(visible.bottom, dest.top, scale_y, source.height),
    );
    (!src.is_empty()).then_some((src, visible))
}

/// `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` bits (`Usage:1`, `RGB_Range:1`,
/// `YCbCr_Matrix:1`, `YCbCr_xvYCC:1`, `Nominal_Range:2`, d3d11.h): of the
/// decoded stream (`Some((bt709, full_range))`: matrix bit, nominal range
/// 16–235 = 1 or 0–255 = 2), or of the output (`None`: RGB full range for
/// playback, all bits 0).
#[cfg_attr(not(windows), allow(dead_code))] // the Windows presenter's
pub fn colour_space_bits(stream: Option<(bool, bool)>) -> u32 {
    match stream {
        None => 0,
        Some((bt709, full_range)) => {
            let matrix = u32::from(bt709) << 2;
            let range = if full_range { 2 } else { 1 };
            matrix | (range << 4)
        }
    }
}

/// Whether egui draws something over the picture area that must stay
/// usable and therefore hides the child: an open popup (combo box) or a
/// visible layer of `Order::Foreground` (a menu) that is not one of the
/// overlays cut out of the child. `layers`: order, whether it is such a
/// cut-out overlay, rectangle.
pub fn menu_over_picture(popup_open: bool, layers: &[(Order, bool, Rect)], area: Rect) -> bool {
    popup_open
        || layers.iter().any(|(order, cut_out, rect)| {
            *order == Order::Foreground && !*cut_out && rect.intersects(area)
        })
}

#[cfg(test)]
#[path = "screen_direct_layout_tests.rs"]
mod tests;
