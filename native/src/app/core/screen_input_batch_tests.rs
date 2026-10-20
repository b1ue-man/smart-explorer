//! More acceptance of the viewer's input (SCR-M7, U25-8): a touch replaces
//! the pointer events egui adds to it, and a frame's events go out in
//! bounded batches.
use eframe::egui::{
    Event, Modifiers, PointerButton as EguiButton, Pos2, TouchId, TouchPhase as EguiPhase,
};

use super::tests::{abs_center, center, everything, geometry, run};
use super::*;

#[test]
fn screen_task_viewer_input_touch_replaces_the_pointer() {
    let mut mapper = InputMapper::new(InputCapabilities {
        touch: true,
        ..everything()
    });
    let touch = |phase| Event::Touch {
        device_id: eframe::egui::TouchDeviceId(1),
        id: TouchId(3),
        phase,
        pos: Pos2::new(600.0, 300.0),
        force: None,
    };
    // egui reports a touch also as pointer events: only the touch goes out.
    let sent = run(
        &mut mapper,
        &[
            touch(EguiPhase::Start),
            Event::PointerMoved(Pos2::new(600.0, 300.0)),
            Event::PointerButton {
                pos: Pos2::new(600.0, 300.0),
                button: EguiButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
    );
    assert_eq!(
        sent,
        [InputEvent::Touch {
            id: 3,
            phase: TouchPhase::Start,
            x: center().0,
            y: center().1
        }]
    );
    // A host without touch gets the pointer instead.
    let mut pointer_only = InputMapper::new(everything());
    let sent = run(
        &mut pointer_only,
        &[
            touch(EguiPhase::Start),
            Event::PointerMoved(Pos2::new(600.0, 300.0)),
        ],
        Modifiers::NONE,
    );
    assert_eq!(sent, [abs_center()]);
}

#[test]
fn screen_task_viewer_input_batches_are_bounded() {
    let mut mapper = InputMapper::new(everything());
    let moves: Vec<Event> = (0..(MAX_INPUT_EVENTS * 2 + 5))
        .map(|index| Event::Text(format!("{index}")))
        .collect();
    mapper.process(&moves, Modifiers::NONE, &geometry());
    let mut sizes = Vec::new();
    while let Some(batch) = mapper.take_batch() {
        assert!(batch.len() <= MAX_INPUT_EVENTS);
        sizes.push(batch.len());
    }
    assert_eq!(sizes, [MAX_INPUT_EVENTS, MAX_INPUT_EVENTS, 5]);
    // Long text is cut at character boundaries into events of at most 4096 bytes.
    let long = "ä".repeat(MAX_TEXT_BYTES);
    let sent = run(&mut mapper, &[Event::Text(long.clone())], Modifiers::NONE);
    assert!(sent.len() >= 2);
    let joined: String = sent
        .iter()
        .map(|event| match event {
            InputEvent::Text { text } => text.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(joined, long);
    assert!(sent.iter().all(|event| matches!(
        event,
        InputEvent::Text { text } if text.len() <= MAX_TEXT_BYTES
    )));
}
