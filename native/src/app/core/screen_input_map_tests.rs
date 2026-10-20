//! Acceptance of the viewer's input (SCR-M7, U25-8): the key table, the
//! pointer, the wheel, typing, the IME and the release of everything held.
use eframe::egui::{
    Event, ImeEvent, Key, Modifiers, MouseWheelUnit, PointerButton as EguiButton, Pos2, Rect, Vec2,
};

use super::super::screen_input_keys::{key_name_of, key_usage_of, local_key, LocalKey};
use super::super::screen_toolbar_ui::{action_label, toolbar_label};
use super::*;
use crate::screen::core::input_model::{key_name, valid_key_usage, SystemAction, TOOLBAR_KEYS};

/// Keys egui knows that have no usage on the wire (decision U25-8).
const UNMAPPED: [Key; 11] = [
    Key::F25,
    Key::F26,
    Key::F27,
    Key::F28,
    Key::F29,
    Key::F30,
    Key::F31,
    Key::F32,
    Key::F33,
    Key::F34,
    Key::F35,
];

pub(super) fn everything() -> InputCapabilities {
    InputCapabilities {
        pointer: true,
        keyboard: true,
        text: true,
        touch: false,
        actions: Vec::new(),
        toolbar_keys: TOOLBAR_KEYS.to_vec(),
    }
}

/// A 1000 × 500 point area showing a 1920 × 1080 source.
pub(super) fn geometry() -> Geometry {
    Geometry {
        rect: Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(1000.0, 500.0)),
        size: FrameSize::new(1920, 1080),
    }
}

/// The pointer position of the middle of the area (pixel 960, 540).
pub(super) fn center() -> (u16, u16) {
    pixel_to_pointer(960, 540, FrameSize::new(1920, 1080))
}

pub(super) fn abs_center() -> InputEvent {
    let (x, y) = center();
    InputEvent::PointerAbs { x, y }
}

fn press(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    }
}

pub(super) fn run(
    mapper: &mut InputMapper,
    events: &[Event],
    modifiers: Modifiers,
) -> Vec<InputEvent> {
    mapper.process(events, modifiers, &geometry());
    let mut out = Vec::new();
    while let Some(batch) = mapper.take_batch() {
        out.extend(batch);
    }
    out
}

fn key_event(name: &str, down: bool) -> InputEvent {
    InputEvent::Key {
        hid_usage: key_usage(name).unwrap(),
        down,
    }
}

#[test]
fn screen_task_input_map_table() {
    // Every egui key has a usage of the wire, or is listed as having none.
    for key in Key::ALL {
        match key_usage_of(*key) {
            Some(usage) => {
                assert!(valid_key_usage(usage), "{key:?} -> {usage:#x}");
                let name = key_name_of(*key).unwrap();
                let back = key_name(usage).and_then(|name| key_usage(&name));
                assert_eq!(back, Some(usage), "{name}");
                assert!(!UNMAPPED.contains(key), "{key:?} has a usage");
            }
            None => assert!(UNMAPPED.contains(key), "{key:?} has no usage"),
        }
    }
    // The decision's table for the keys that are not obvious.
    for (key, name) in [
        (Key::Plus, "equal"),
        (Key::Equals, "equal"),
        (Key::Colon, "semicolon"),
        (Key::Pipe, "backslash"),
        (Key::Questionmark, "slash"),
        (Key::Backtick, "grave"),
        (Key::OpenBracket, "bracket_left"),
        (Key::Num0, "0"),
        (Key::F24, "f24"),
        (Key::PageDown, "page_down"),
        (Key::Copy, "copy"),
    ] {
        assert_eq!(key_name_of(key), Some(name), "{key:?}");
    }
    // Every key of the toolbar and every action has a label and a usage.
    for usage in TOOLBAR_KEYS {
        assert!(valid_key_usage(usage), "{usage:#x}");
        assert!(toolbar_label(usage).is_some(), "{usage:#x} has no label");
    }
    for action in [
        SystemAction::Back,
        SystemAction::Home,
        SystemAction::Recents,
        SystemAction::Notifications,
        SystemAction::QuickSettings,
        SystemAction::LockScreen,
        SystemAction::PowerDialog,
    ] {
        assert!(!action_label(action).is_empty());
    }
}

#[test]
fn screen_task_ime_commit_sends_text() {
    let mut mapper = InputMapper::new(everything());
    // A composition is shown, not sent; its commit goes out as text.
    let sent = run(
        &mut mapper,
        &[Event::Ime(ImeEvent::Preedit("か".into()))],
        Modifiers::NONE,
    );
    assert!(sent.is_empty());
    assert_eq!(mapper.preedit.as_deref(), Some("か"));
    let sent = run(
        &mut mapper,
        &[Event::Ime(ImeEvent::Commit("漢字".into()))],
        Modifiers::NONE,
    );
    assert_eq!(
        sent,
        [InputEvent::Text {
            text: "漢字".into()
        }]
    );
    assert!(mapper.preedit.is_none());
    // A host that takes no text gets none.
    let mut keys_only = InputMapper::new(InputCapabilities {
        text: false,
        ..everything()
    });
    let sent = run(
        &mut keys_only,
        &[Event::Ime(ImeEvent::Commit("x".into()))],
        Modifiers::NONE,
    );
    assert!(sent.is_empty());
}

#[test]
fn screen_task_viewer_input_pointer_wheel_and_buttons() {
    let mut mapper = InputMapper::new(everything());
    // The middle of the area is the middle of the source; outside sends nothing.
    let middle = Pos2::new(600.0, 300.0);
    let sent = run(
        &mut mapper,
        &[
            Event::PointerMoved(Pos2::new(5.0, 5.0)),
            Event::PointerMoved(middle),
        ],
        Modifiers::NONE,
    );
    assert_eq!(sent, [abs_center()]);
    let corner = Pos2::new(100.0, 50.0);
    let sent = run(&mut mapper, &[Event::PointerMoved(corner)], Modifiers::NONE);
    assert_eq!(sent, [InputEvent::PointerAbs { x: 0, y: 0 }]);

    // Buttons: the press moves the pointer first; a release beside the
    // picture still arrives (on the edge); a press beside it does not.
    let click = |pressed| Event::PointerButton {
        pos: middle,
        button: EguiButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let sent = run(&mut mapper, &[click(true), click(false)], Modifiers::NONE);
    assert_eq!(
        sent,
        [
            abs_center(),
            InputEvent::Button {
                button: PointerButton::Right,
                down: true
            },
            InputEvent::Button {
                button: PointerButton::Right,
                down: false
            },
        ]
    );
    let beside = Event::PointerButton {
        pos: Pos2::new(1.0, 1.0),
        button: EguiButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    };
    assert!(run(&mut mapper, &[beside], Modifiers::NONE).is_empty());
    let back = |pressed| Event::PointerButton {
        pos: middle,
        button: EguiButton::Extra1,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let sent = run(&mut mapper, &[back(true)], Modifiers::NONE);
    assert!(sent.contains(&InputEvent::Button {
        button: PointerButton::Back,
        down: true
    }));

    // The wheel: up is positive, one notch is 120; points are 40 to a notch
    // and the rest of a notch is kept for the next event.
    let wheel = |unit, delta: Vec2| Event::MouseWheel {
        unit,
        delta,
        modifiers: Modifiers::NONE,
    };
    let sent = run(
        &mut mapper,
        &[wheel(MouseWheelUnit::Line, Vec2::new(0.0, 1.0))],
        Modifiers::NONE,
    );
    assert_eq!(sent, [InputEvent::Wheel { dx: 0, dy: 120 }]);
    let sent = run(
        &mut mapper,
        &[wheel(MouseWheelUnit::Line, Vec2::new(1.0, -2.0))],
        Modifiers::NONE,
    );
    assert_eq!(sent, [InputEvent::Wheel { dx: -120, dy: -240 }]);
    let sent = run(
        &mut mapper,
        &[
            wheel(MouseWheelUnit::Point, Vec2::new(0.0, 10.0)),
            wheel(MouseWheelUnit::Point, Vec2::new(0.0, 10.0)),
            wheel(MouseWheelUnit::Point, Vec2::new(0.0, 10.0)),
        ],
        Modifiers::NONE,
    );
    assert_eq!(
        sent,
        [
            InputEvent::Wheel { dx: 0, dy: 30 },
            InputEvent::Wheel { dx: 0, dy: 30 },
            InputEvent::Wheel { dx: 0, dy: 30 },
        ]
    );
}

#[test]
fn screen_task_viewer_input_text_keys_and_shortcuts() {
    let mut mapper = InputMapper::new(everything());
    // Typing: the host gets the character, not the key; no repeat events.
    let sent = run(
        &mut mapper,
        &[
            press(Key::A, true, Modifiers::NONE),
            Event::Text("a".into()),
            press(Key::A, false, Modifiers::NONE),
            press(Key::Enter, true, Modifiers::NONE),
            press(Key::Enter, false, Modifiers::NONE),
        ],
        Modifiers::NONE,
    );
    assert_eq!(
        sent,
        [
            InputEvent::Text { text: "a".into() },
            key_event("enter", true),
            key_event("enter", false),
        ]
    );
    // Held keys repeat on the host: a repeat adds no press.
    let repeat = Event::Key {
        key: Key::ArrowDown,
        physical_key: None,
        pressed: true,
        repeat: true,
        modifiers: Modifiers::NONE,
    };
    let sent = run(
        &mut mapper,
        &[press(Key::ArrowDown, true, Modifiers::NONE), repeat],
        Modifiers::NONE,
    );
    assert_eq!(sent, [key_event("down", true)]);
    let sent = run(
        &mut mapper,
        &[press(Key::ArrowDown, false, Modifiers::NONE)],
        Modifiers::NONE,
    );
    assert_eq!(sent, [key_event("down", false)]);

    // Shortcuts go as keys with the modifier before and after.
    let sent = run(
        &mut mapper,
        &[press(Key::S, true, ctrl()), press(Key::S, false, ctrl())],
        ctrl(),
    );
    assert_eq!(
        sent,
        [
            key_event("ctrl_left", true),
            key_event("s", true),
            key_event("s", false),
        ]
    );
    let sent = run(&mut mapper, &[], Modifiers::NONE);
    assert_eq!(sent, [key_event("ctrl_left", false)]);
    // Ctrl+C arrives from egui as `Copy`.
    let sent = run(&mut mapper, &[Event::Copy], Modifiers::NONE);
    assert_eq!(
        sent,
        [
            key_event("ctrl_left", true),
            key_event("c", true),
            key_event("c", false),
            key_event("ctrl_left", false),
        ]
    );
    // Text with Ctrl held is a shortcut's side effect: dropped.
    let sent = run(
        &mut mapper,
        &[press(Key::V, true, ctrl()), Event::Text("v".into())],
        ctrl(),
    );
    assert!(!sent
        .iter()
        .any(|event| matches!(event, InputEvent::Text { .. })));

    // The viewer's own shortcuts do not reach the host, press or release.
    let both = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    assert_eq!(
        local_key(Key::F11, Modifiers::NONE),
        Some(LocalKey::Fullscreen)
    );
    assert_eq!(local_key(Key::S, both), Some(LocalKey::Stats));
    assert_eq!(local_key(Key::K, both), Some(LocalKey::ToggleControl));
    assert_eq!(local_key(Key::S, ctrl()), None);
    let mut quiet = InputMapper::new(everything());
    let sent = run(
        &mut quiet,
        &[
            press(Key::F11, true, Modifiers::NONE),
            press(Key::F11, false, Modifiers::NONE),
        ],
        Modifiers::NONE,
    );
    assert!(sent.is_empty());

    // Keys without a usage are counted and dropped.
    let sent = run(
        &mut quiet,
        &[press(Key::F30, true, Modifiers::NONE)],
        Modifiers::NONE,
    );
    assert!(sent.is_empty());
    assert_eq!(quiet.unmapped_keys, 1);
}

#[test]
fn screen_task_viewer_input_releases_everything_held() {
    let mut mapper = InputMapper::new(everything());
    let middle = Pos2::new(600.0, 300.0);
    let sent = run(
        &mut mapper,
        &[
            press(Key::Z, true, ctrl()),
            Event::PointerButton {
                pos: middle,
                button: EguiButton::Primary,
                pressed: true,
                modifiers: ctrl(),
            },
        ],
        ctrl(),
    );
    assert!(sent.contains(&key_event("ctrl_left", true)));
    // Control ends: everything still down goes up, newest first, so the
    // modifier is the last to be lifted.
    mapper.release_all();
    let mut released = Vec::new();
    while let Some(batch) = mapper.take_batch() {
        released.extend(batch);
    }
    assert_eq!(
        released,
        [
            InputEvent::Button {
                button: PointerButton::Left,
                down: false
            },
            key_event("z", false),
            key_event("ctrl_left", false),
        ]
    );
    // Nothing is left to lift, and the next press starts from a clean state.
    mapper.release_all();
    assert!(mapper.take_batch().is_none());
    let sent = run(
        &mut mapper,
        &[press(Key::Escape, true, Modifiers::NONE)],
        Modifiers::NONE,
    );
    assert_eq!(sent, [key_event("escape", true)]);
    // A window that loses the focus lifts everything too.
    let sent = run(&mut mapper, &[Event::WindowFocused(false)], Modifiers::NONE);
    assert_eq!(sent, [key_event("escape", false)]);
}
