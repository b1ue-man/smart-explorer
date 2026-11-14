//! The host's pointer drawn over the picture (plan 2026-11-14, package P6,
//! decision C5): shape and position come as control frames, not inside the
//! picture, so a pointer movement does not wait for capture, encoding,
//! transport and decoding of a picture. A view that only watches draws it at
//! the position the host reported; a view that steers (its input goes to the
//! host) draws it at the local pointer while that is over the picture (the
//! host follows the local pointer anyway) and hides the system pointer there,
//! so there is one pointer, not two. The overlay is an egui image over the
//! picture, so it is the same on the wgpu and the glow renderer.
use std::sync::Arc;

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Vec2};

use super::screen_session::ViewerSession;
use crate::screen::core::cursor_overlay::CursorShape;

/// The texture of the shape shown last, kept in the context per viewport.
/// The shape itself names it, not its id: every host process counts its
/// shapes from one, so after a new connection or another source the same id
/// is another picture.
#[derive(Clone)]
struct ShapeTexture {
    shape: Arc<CursorShape>,
    texture: egui::TextureHandle,
}

fn texture(ctx: &egui::Context, shape: &Arc<CursorShape>) -> egui::TextureHandle {
    let key = egui::Id::new(("screen_cursor_texture", ctx.viewport_id()));
    if let Some(held) = ctx.data(|data| data.get_temp::<ShapeTexture>(key)) {
        if Arc::ptr_eq(&held.shape, shape) {
            return held.texture;
        }
    }
    let image = egui::ColorImage::from_rgba_premultiplied(
        [usize::from(shape.width), usize::from(shape.height)],
        &shape.rgba,
    );
    let texture = ctx.load_texture("screen-cursor", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| {
        data.insert_temp(
            key,
            ShapeTexture {
                shape: shape.clone(),
                texture: texture.clone(),
            },
        )
    });
    texture
}

/// Where the pointer's hot spot lies on the screen and the pointer's top-left
/// corner and size, for a picture drawn into `dest`.
pub(super) fn place(dest: Rect, shape: &CursorShape, area: (f32, f32), hot_spot: HotSpot) -> Rect {
    let scale = Vec2::new(dest.width() / area.0, dest.height() / area.1);
    let thick = f32::from(shape.shrink);
    let at = match hot_spot {
        HotSpot::Host { x, y } => dest.min + Vec2::new(x as f32 * scale.x, y as f32 * scale.y),
        HotSpot::Local(position) => position,
    };
    let origin = at
        - Vec2::new(
            f32::from(shape.hot_x) * thick * scale.x,
            f32::from(shape.hot_y) * thick * scale.y,
        );
    let size = Vec2::new(
        f32::from(shape.width) * thick * scale.x,
        f32::from(shape.height) * thick * scale.y,
    );
    Rect::from_min_size(origin, size)
}

/// Which position the pointer is drawn at.
pub(super) enum HotSpot {
    /// The host's position, in pixels of the captured area.
    Host { x: i32, y: i32 },
    /// The local pointer, in points of the window.
    Local(Pos2),
}

/// Draws the host's pointer over the picture shown in `dest`. Nothing is
/// drawn until the host sent a visible pointer, so a host that keeps the
/// pointer in the picture shows it as before. `steering`: this view's input
/// goes to the host right now and its pointer is over the picture, not over
/// something that covers it.
pub(super) fn paint(painter: &egui::Painter, dest: Rect, session: &ViewerSession, steering: bool) {
    let (shape, position) = {
        let info = session.info();
        let Some((_, position)) = info.cursor.visible() else {
            return;
        };
        let position = *position;
        let Some(shape) = info.cursor.shape.clone() else {
            return;
        };
        (shape, position)
    };
    let ctx = painter.ctx();
    let local = ctx
        .pointer_hover_pos()
        .filter(|pointer| steering && dest.contains(*pointer));
    let hot_spot = match local {
        Some(pointer) => HotSpot::Local(pointer),
        None => HotSpot::Host {
            x: position.x,
            y: position.y,
        },
    };
    let area = (position.area.width as f32, position.area.height as f32);
    let rect = place(dest, &shape, area, hot_spot);
    if local.is_some() {
        ctx.set_cursor_icon(CursorIcon::None);
    }
    painter.image(
        texture(ctx, &shape).id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

#[cfg(test)]
#[path = "screen_cursor_tests.rs"]
mod tests;
