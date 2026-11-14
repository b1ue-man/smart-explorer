//! The pointer events of the Android viewer (plan 2026-11-14, package P6).
use super::*;
use crate::screen::core::pixel_convert::FrameSize;

fn shape() -> CursorShape {
    CursorShape {
        id: 4,
        width: 2,
        height: 1,
        hot_x: 1,
        hot_y: 0,
        shrink: 2,
        rgba: vec![9; 8],
    }
}

#[test]
fn verbindungen_task_zeiger_mobile_events_carry_what_the_app_draws() {
    let events = CursorEvents::default();
    let fields = events.shape(&shape());
    assert_eq!(fields["id"], 4);
    assert_eq!(fields["shrink"], 2);
    assert_eq!(fields["rgbaBase64"], STANDARD.encode([9u8; 8]));
    let position = position_fields(&CursorPosition {
        shape_id: 4,
        visible: true,
        x: -3,
        y: 300,
        area: FrameSize::new(800, 600),
    });
    assert_eq!(
        position,
        json!({"shapeId": 4, "visible": true, "x": -3, "y": 300, "areaWidth": 800, "areaHeight": 600})
    );
}

#[test]
fn verbindungen_task_zeiger_mobile_shape_repeats_only_after_the_pause() {
    let events = CursorEvents::default();
    assert!(
        events.repeat().is_none(),
        "nothing to repeat before a shape"
    );
    let fields = events.shape(&shape());
    assert!(events.repeat().is_none(), "just sent");
    // The shape is held with the time it was sent: a later one repeats it.
    events
        .shape
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
        .unwrap()
        .1 = Instant::now() - REPEAT;
    assert_eq!(events.repeat(), Some(fields));
    assert!(events.repeat().is_none(), "repeated once, then quiet again");
    // A new connection forgets the shape of the one before.
    events
        .shape
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
        .unwrap()
        .1 = Instant::now() - REPEAT;
    events.clear();
    assert!(events.repeat().is_none(), "nothing held after a new start");
}

#[test]
fn verbindungen_task_zeiger_mobile_hidden_position_is_an_event_the_app_reads() {
    // What the host sends first after a request and what a new connection
    // starts with: the app's decoder needs a non-empty area and hides.
    let fields = position_fields(&CursorPosition::hidden());
    assert_eq!(fields["visible"], false);
    assert_eq!(fields["areaWidth"], 1);
    assert_eq!(fields["areaHeight"], 1);
}
