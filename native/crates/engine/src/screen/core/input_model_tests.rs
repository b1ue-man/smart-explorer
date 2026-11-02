use super::*;

fn key(name: &str, down: bool) -> InputEvent {
    InputEvent::Key {
        hid_usage: key_usage(name).unwrap(),
        down,
    }
}

fn touch(id: u8, phase: TouchPhase, x: u16, y: u16) -> InputEvent {
    InputEvent::Touch { id, phase, x, y }
}

fn button(button: PointerButton, down: bool) -> InputEvent {
    InputEvent::Button { button, down }
}

#[test]
fn screen_task_input_release_on_end() {
    let mut pressed = PressedInputs::default();
    let applied = [
        key("ctrl_left", true),
        key("c", true),
        key("c", true), // key repeat
        button(PointerButton::Left, true),
        touch(1, TouchPhase::Start, 10, 20),
        touch(1, TouchPhase::Move, 30, 40),
        key("a", true),
        key("a", false),
        touch(2, TouchPhase::Start, 5, 5),
        touch(2, TouchPhase::End, 5, 5),
        button(PointerButton::Right, true),
        button(PointerButton::Right, false),
        InputEvent::PointerAbs { x: 9, y: 9 },
        InputEvent::Wheel { dx: 0, dy: -120 },
        InputEvent::Text { text: "x".into() },
        InputEvent::Action {
            action: SystemAction::Back,
        },
    ];
    for event in &applied {
        pressed.observe(event);
    }
    // Everything still held goes up, newest first, so the modifier last.
    let release = pressed.release_all();
    assert_eq!(
        release,
        [
            touch(1, TouchPhase::Cancel, 30, 40),
            button(PointerButton::Left, false),
            key("c", false),
            key("ctrl_left", false),
        ]
    );
    assert!(pressed.is_empty());
    assert!(pressed.release_all().is_empty());
    // The release travels as an ordinary `INPUT` batch.
    assert_eq!(
        decode_batch(&encode_batch(&release).unwrap()).unwrap(),
        release
    );
    // A release the host applied leaves nothing held either.
    for event in &release {
        pressed.observe(event);
    }
    assert!(pressed.is_empty());
}

#[test]
fn screen_task_input_key_names_and_pointer_positions() {
    let expected = [
        ("a", 0x04),
        ("z", 0x1D),
        ("1", 0x1E),
        ("9", 0x26),
        ("0", 0x27),
        ("enter", 0x28),
        ("f1", 0x3A),
        ("f12", 0x45),
        ("f13", 0x68),
        ("f24", 0x73),
        ("print_screen", 0x46),
        ("super_left", 0xE3),
        ("menu", 0x65),
        ("play_pause", 0x80CD),
        ("browser_back", 0x8224),
    ];
    for (name, usage) in expected {
        assert_eq!(key_usage(name), Some(usage), "{name}");
        assert_eq!(key_name(usage).as_deref(), Some(name));
    }
    for name in ["f0", "f25", "fn", "A", "", "ctrl"] {
        assert_eq!(key_usage(name), None, "{name}");
    }
    // Every toolbar key has a name and passes the host's check.
    for usage in TOOLBAR_KEYS {
        assert!(valid_key_usage(usage));
        assert_eq!(key_usage(&key_name(usage).unwrap()), Some(usage));
    }
    assert!(!valid_key_usage(0x03) && !valid_key_usage(0xE8) && !valid_key_usage(0x8400));
    assert!(encode_batch(&[InputEvent::Key {
        hid_usage: 0x0100,
        down: true
    }])
    .is_err());

    // Positions 0–65535 cover the source from edge to edge.
    let size = FrameSize::new(1920, 1080);
    assert_eq!(pointer_to_pixel(0, 0, size), (0, 0));
    assert_eq!(pointer_to_pixel(65_535, 65_535, size), (1919, 1079));
    assert_eq!(pixel_to_pointer(1919, 1079, size), (65_535, 65_535));
    assert_eq!(pixel_to_pointer(5000, 0, size), (65_535, 0));
    for pixel in [0, 1, 959, 960, 1918, 1919] {
        let (x, _) = pixel_to_pointer(pixel, 0, size);
        assert_eq!(pointer_to_pixel(x, 0, size).0, pixel);
    }
    assert_eq!(pixel_to_pointer(0, 0, FrameSize::new(1, 1)), (0, 0));

    // The JSON form of the bridge and the CLI.
    let json = serde_json::to_string(&button(PointerButton::Middle, true)).unwrap();
    assert_eq!(json, r#"{"type":"button","button":"middle","down":true}"#);
    let parsed: InputEvent =
        serde_json::from_str(r#"{"type":"action","action":"lock_screen"}"#).unwrap();
    assert_eq!(
        parsed,
        InputEvent::Action {
            action: SystemAction::LockScreen
        }
    );
}
