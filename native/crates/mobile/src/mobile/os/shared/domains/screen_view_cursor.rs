//! The host's pointer for the Android viewer (plan 2026-11-14, package P6):
//! the `screenView` events `cursorShape` and `cursorPosition`. A host that
//! can leave the pointer out of the picture sends it as control frames; a
//! host that cannot keeps it in the picture and no event is emitted. The
//! shape is large (up to 64 KiB) and the app may subscribe after the first
//! one, so it is sent again while positions flow, at most every two seconds.
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

use crate::screen::core::cursor_overlay::{CursorPosition, CursorShape};

/// Time after which the last shape is sent again.
const REPEAT: Duration = Duration::from_secs(2);

/// The last shape of a session, as event fields, and when it was sent.
#[derive(Default)]
pub(super) struct CursorEvents {
    shape: Mutex<Option<(Value, Instant)>>,
}

impl CursorEvents {
    /// The fields of a new shape's event; remembers them for the repeat.
    pub(super) fn shape(&self, shape: &CursorShape) -> Value {
        let fields = json!({
            "id": shape.id,
            "width": shape.width,
            "height": shape.height,
            "hotX": shape.hot_x,
            "hotY": shape.hot_y,
            "shrink": shape.shrink,
            "rgbaBase64": STANDARD.encode(&shape.rgba),
        });
        *self.shape.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((fields.clone(), Instant::now()));
        fields
    }

    /// Forgets the held shape: a new connection starts without a pointer.
    pub(super) fn clear(&self) {
        *self.shape.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// The last shape's fields once `REPEAT` passed since it was sent.
    pub(super) fn repeat(&self) -> Option<Value> {
        let mut held = self.shape.lock().unwrap_or_else(PoisonError::into_inner);
        let (fields, sent) = held.as_mut()?;
        (sent.elapsed() >= REPEAT).then(|| {
            *sent = Instant::now();
            fields.clone()
        })
    }
}

/// The fields of a `cursorPosition` event.
pub(super) fn position_fields(position: &CursorPosition) -> Value {
    json!({
        "shapeId": position.shape_id,
        "visible": position.visible,
        "x": position.x,
        "y": position.y,
        "areaWidth": position.area.width,
        "areaHeight": position.area.height,
    })
}

#[cfg(test)]
#[path = "screen_view_cursor_tests.rs"]
mod tests;
