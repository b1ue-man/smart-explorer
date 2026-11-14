//! The X11 pointer with the overlay (plan 2026-11-14, package P6): the
//! picture keeps its own pixels once the viewer draws the pointer itself, and
//! shape and position reach the overlay instead.
use super::*;
use crate::screen::core::cursor_overlay::{CursorUpdate, WorkerCursor};

fn cursor() -> CursorImage {
    // 2 x 1 image whose top-left corner is at (10, 20) on the root window.
    CursorImage {
        x: 10,
        y: 20,
        width: 2,
        height: 1,
        argb: vec![0xFFFF_0000, 0x8080_8080],
    }
}

#[test]
fn verbindungen_task_zeiger_x11_blends_without_the_overlay_and_not_with_it() {
    let size = FrameSize::new(40, 30);
    let origin = (5, 5);
    let mut mixed = vec![100u8; size.bgra_len()];
    let blended = WorkerCursor::new();
    blended.set_supported();
    deliver(&blended, &mut mixed, size, origin, &cursor(), (1, 0), 7);
    assert!(
        mixed.iter().any(|byte| *byte != 100),
        "the pointer is in the picture"
    );
    assert!(blended.take().is_empty(), "nothing goes to the overlay");

    let mut kept = vec![100u8; size.bgra_len()];
    let overlay = WorkerCursor::new();
    overlay.set_supported();
    assert!(overlay.set_separate(true));
    deliver(&overlay, &mut kept, size, origin, &cursor(), (1, 0), 7);
    assert!(
        kept.iter().all(|byte| *byte == 100),
        "the picture stays as captured"
    );
    let updates = overlay.take();
    let [CursorUpdate::Shape(shape), CursorUpdate::Position(position)] = updates.as_slice() else {
        panic!("shape first, then position: {updates:?}");
    };
    assert_eq!(
        (shape.width, shape.height, shape.hot_x, shape.hot_y),
        (2, 1, 1, 0)
    );
    // Hot spot (11, 20) on the root, the area starts at (5, 5): (6, 15) of it.
    assert_eq!((position.x, position.y, position.visible), (6, 15, true));
    assert_eq!(position.shape_id, shape.id);
    assert_eq!(position.area, size);
}

#[test]
fn verbindungen_task_zeiger_x11_sends_the_shape_again_only_when_its_serial_changes() {
    let size = FrameSize::new(40, 30);
    let mut pixels = vec![0u8; size.bgra_len()];
    let overlay = WorkerCursor::new();
    overlay.set_supported();
    assert!(overlay.set_separate(true));
    deliver(&overlay, &mut pixels, size, (0, 0), &cursor(), (0, 0), 1);
    assert_eq!(overlay.take().len(), 2);
    // Same serial, the pointer moved: only the position.
    let mut moved = cursor();
    moved.x += 3;
    deliver(&overlay, &mut pixels, size, (0, 0), &moved, (0, 0), 1);
    let updates = overlay.take();
    assert!(
        matches!(updates.as_slice(), [CursorUpdate::Position(_)]),
        "{updates:?}"
    );
    // Same serial, same place: nothing.
    deliver(&overlay, &mut pixels, size, (0, 0), &moved, (0, 0), 1);
    assert!(overlay.take().is_empty());
    // A new serial: the shape again.
    deliver(&overlay, &mut pixels, size, (0, 0), &moved, (0, 0), 2);
    assert!(matches!(
        overlay.take().as_slice(),
        [CursorUpdate::Shape(_), CursorUpdate::Position(_)]
    ));
    // A pointer outside the area is hidden for the viewer.
    deliver(&overlay, &mut pixels, size, (1000, 1000), &moved, (0, 0), 2);
    let outside = overlay.take();
    assert!(
        matches!(outside.as_slice(), [CursorUpdate::Position(p)] if !p.visible),
        "{outside:?}"
    );
}
