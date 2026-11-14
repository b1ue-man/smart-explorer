//! Acceptance of the pointer overlay of the desktop viewer (plan 2026-11-14,
//! package P6): it appears over the picture where the host's pointer is, in
//! the layout frame of egui, and only once the host sent a visible pointer.
use eframe::egui;

use super::super::screen_session::{Shared, ViewerSession};
use super::{paint, place, HotSpot};
use crate::screen::core::cursor_overlay::{CursorPosition, CursorShape};
use crate::screen::core::pixel_convert::FrameSize;

fn shape(shrink: u16) -> CursorShape {
    CursorShape {
        id: 4,
        width: 8,
        height: 12,
        hot_x: 2,
        hot_y: 3,
        shrink,
        rgba: vec![255; 8 * 12 * 4],
    }
}

fn position(visible: bool) -> CursorPosition {
    CursorPosition {
        shape_id: 4,
        visible,
        x: 400,
        y: 300,
        area: FrameSize::new(800, 600),
    }
}

/// One frame in which the overlay paints over `dest`: the areas of the
/// shapes it drew and the system pointer the frame asks for. `pointer`: where
/// the local pointer is; `steering`: this view's input goes to the host.
fn frame(
    dest: egui::Rect,
    shared: &std::sync::Arc<Shared>,
    pointer: Option<egui::Pos2>,
    steering: bool,
) -> (Vec<egui::Rect>, egui::CursorIcon) {
    let session = ViewerSession::fixture(shared.clone());
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400.0, 300.0),
        )),
        events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let painter = ui.painter().clone();
            paint(&painter, dest, &session, steering);
        });
    });
    let shapes = output
        .shapes
        .iter()
        .map(|clipped| clipped.shape.visual_bounding_rect())
        .collect();
    (shapes, output.platform_output.cursor_icon)
}

/// The areas of the shapes one frame of a watching view drew.
fn drawn(dest: egui::Rect, shared: &std::sync::Arc<Shared>) -> Vec<egui::Rect> {
    frame(dest, shared, None, false).0
}

fn near(rects: &[egui::Rect], expected: egui::Rect) -> bool {
    rects
        .iter()
        .any(|rect| rect.min.distance(expected.min) < 0.5 && rect.max.distance(expected.max) < 0.5)
}

#[test]
fn verbindungen_task_zeiger_overlay_is_drawn_where_the_host_pointer_is() {
    let dest = egui::Rect::from_min_size(egui::pos2(20.0, 10.0), egui::vec2(200.0, 150.0));
    let shared = Shared::new(false, 0);
    shared.update(|info| {
        info.cursor.set_shape(shape(1));
        info.cursor.set_position(position(true));
    });
    // A 800 x 600 area drawn at 200 x 150 points: a quarter, the hot spot
    // (400, 300) is the middle of the picture, the hot spot of the shape
    // (2, 3) lies a quarter of its pixels from the corner.
    let expected =
        egui::Rect::from_min_size(egui::pos2(120.0 - 0.5, 85.0 - 0.75), egui::vec2(2.0, 3.0));
    assert!(
        near(&drawn(dest, &shared), expected),
        "the overlay pointer is drawn at the host position"
    );
}

/// What the user asked for (2026-11-14: two pointers, the own and the
/// host's): a view that steers shows exactly one pointer over the picture,
/// the host's shape at the local pointer, and no system pointer beside it.
#[test]
fn verbindungen_task_zeiger_a_steering_view_shows_one_pointer_at_the_local_position() {
    let dest = egui::Rect::from_min_size(egui::pos2(20.0, 10.0), egui::vec2(200.0, 150.0));
    let shared = Shared::new(false, 0);
    shared.update(|info| {
        info.cursor.set_shape(shape(1));
        info.cursor.set_position(position(true));
    });
    let pointer = egui::pos2(60.0, 40.0);
    let at_pointer =
        egui::Rect::from_min_size(egui::pos2(60.0 - 0.5, 40.0 - 0.75), egui::vec2(2.0, 3.0));
    let at_host =
        egui::Rect::from_min_size(egui::pos2(120.0 - 0.5, 85.0 - 0.75), egui::vec2(2.0, 3.0));
    let (shapes, icon) = frame(dest, &shared, Some(pointer), true);
    assert!(near(&shapes, at_pointer), "drawn at the local pointer");
    assert!(!near(&shapes, at_host), "not a second time at the host's");
    assert_eq!(icon, egui::CursorIcon::None, "the system pointer is hidden");

    // The same pointer over the picture of a view that only watches: the
    // host's pointer stays where the host has it, the own pointer stays.
    let (shapes, icon) = frame(dest, &shared, Some(pointer), false);
    assert!(near(&shapes, at_host));
    assert!(!near(&shapes, at_pointer));
    assert_ne!(icon, egui::CursorIcon::None);

    // Steering with the pointer beside the picture: the system pointer stays.
    let (shapes, icon) = frame(dest, &shared, Some(egui::pos2(300.0, 250.0)), true);
    assert!(near(&shapes, at_host));
    assert_ne!(icon, egui::CursorIcon::None);

    // A host that keeps the pointer in its picture sends none: nothing is
    // drawn and the system pointer stays, as before the overlay existed.
    let plain = Shared::new(false, 0);
    let (_, icon) = frame(dest, &plain, Some(pointer), true);
    assert_ne!(icon, egui::CursorIcon::None);
}

#[test]
fn verbindungen_task_zeiger_nothing_is_drawn_for_a_hidden_or_unknown_pointer() {
    let dest = egui::Rect::from_min_size(egui::pos2(20.0, 10.0), egui::vec2(200.0, 150.0));
    let none = Shared::new(false, 0);
    let baseline = drawn(dest, &none).len();
    let hidden = Shared::new(false, 0);
    hidden.update(|info| {
        info.cursor.set_shape(shape(1));
        info.cursor.set_position(position(false));
    });
    assert_eq!(drawn(dest, &hidden).len(), baseline, "a hidden pointer");
    let other = Shared::new(false, 0);
    other.update(|info| {
        info.cursor.set_shape(shape(1));
        let mut moved = position(true);
        moved.shape_id = 9;
        info.cursor.set_position(moved);
    });
    assert_eq!(
        drawn(dest, &other).len(),
        baseline,
        "a position of another shape"
    );
}

#[test]
fn verbindungen_task_zeiger_local_pointer_places_the_hot_spot_under_the_mouse() {
    let dest = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0));
    // Thinned out by 2: the shape draws twice as large, its hot spot too.
    let rect = place(
        dest,
        &shape(2),
        (800.0, 600.0),
        HotSpot::Local(egui::pos2(100.0, 50.0)),
    );
    assert_eq!(rect.min, egui::pos2(100.0 - 2.0, 50.0 - 3.0));
    assert_eq!(rect.size(), egui::vec2(8.0, 12.0));
    // The host's position is mapped onto the picture the same way.
    let host = place(
        dest,
        &shape(1),
        (800.0, 600.0),
        HotSpot::Host { x: 800, y: 600 },
    );
    assert_eq!(host.min, egui::pos2(400.0 - 1.0, 300.0 - 1.5));
}
