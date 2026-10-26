//! One image geometry for drawing, native-pixel panning, visibility and input.
use eframe::egui::{Pos2,Rect,Vec2};
use super::screen_prefs::Fit;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::input_model::pixel_to_pointer;

/// Where the picture lies on the screen and how large its source is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    /// The picture's area in points.
    pub rect: Rect,
    /// Pixels of the source.
    pub size: FrameSize,
}

impl Geometry {
    /// The pointer position 0–65535 of `pos`, if it lies on the picture.
    pub(crate) fn pointer_at(&self, pos: Pos2) -> Option<(u16, u16)> {
        self.rect.contains(pos).then(|| self.clamped(pos))
    }

    /// The same, with positions beyond the picture on its edge.
    pub(crate) fn clamped(&self, pos: Pos2) -> (u16, u16) {
        let axis = |value: f32, start: f32, length: f32, pixels: u32| {
            let share = if length > 0.0 {
                ((value - start) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ((share * pixels as f32) as u32).min(pixels.saturating_sub(1))
        };
        let px = axis(pos.x, self.rect.min.x, self.rect.width(), self.size.width);
        let py = axis(pos.y, self.rect.min.y, self.rect.height(), self.size.height);
        pixel_to_pointer(px, py, self.size)
    }
}

/// Where the picture of `size` lies in `area` (points). `pan` shifts a 1:1
/// picture that is larger than the area.
pub(crate) fn image_rect(area: Rect, size: FrameSize, fit: Fit, ppp: f32, pan: Vec2) -> Rect {
    let (w, h) = (size.width as f32, size.height as f32);
    let dest = match fit {
        Fit::Fit => {
            let scale = (area.width() / w).min(area.height() / h);
            Vec2::new(w * scale, h * scale)
        }
        Fit::Actual => Vec2::new(w / ppp, h / ppp),
    };
    let place = |dest: f32, room: f32, start: f32, pan: f32| {
        if dest <= room {
            start + (room - dest) / 2.0
        } else {
            start - pan.clamp(0.0, dest - room)
        }
    };
    let x = place(dest.x, area.width(), area.min.x, pan.x);
    let y = place(dest.y, area.height(), area.min.y, pan.y);
    let start = if fit == Fit::Actual { Pos2::new((x * ppp).round() / ppp, (y * ppp).round() / ppp) }
        else { Pos2::new(x, y) };
    Rect::from_min_size(start, dest)
}


/// egui 0.29 clamps GPU callback viewports to the framebuffer. Preserve the
/// original image mapping inside that clipped viewport, including panning.
pub(crate) fn viewport_uv(rect: Rect, ppp: f32, screen: [u32;2]) -> Option<[f32;4]> {
    let min = rect.min*ppp;
    let size = rect.size()*ppp;
    if size.x <= 0.0 || size.y <= 0.0 { return None; }
    let left = min.x.round().clamp(0.0,screen[0] as f32);
    let top = min.y.round().clamp(0.0,screen[1] as f32);
    let right = (rect.max.x*ppp).round().clamp(left,screen[0] as f32);
    let bottom = (rect.max.y*ppp).round().clamp(top,screen[1] as f32);
    Some([(left-min.x)/size.x,(top-min.y)/size.y,(right-left)/size.x,(bottom-top)/size.y])
}
