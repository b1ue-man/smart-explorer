//! `se screen input` without a device (SCR-M6): the options become exactly
//! the events the host receives.
use super::*;

#[test]
fn screen_task_cli_input_events() {
    let size = Some(FrameSize::new(1920, 1080));
    let click = action_of(None, None, Some("960,540"), None, None, None).unwrap();
    let events = events(&click, size).unwrap();
    match events[0] {
        InputEvent::PointerAbs { x, y } => {
            assert!((32_770..=32_800).contains(&x), "{x}");
            assert!((32_770..=32_810).contains(&y), "{y}");
        }
        ref other => panic!("{other:?}"),
    }
    assert_eq!(events.len(), 3);
    let middle = action_of(None, None, Some("50%,50%"), None, None, None).unwrap();
    assert_eq!(
        super::events(&middle, None).unwrap()[0],
        InputEvent::PointerAbs {
            x: 32_768,
            y: 32_768
        }
    );
    // Pixels need the source size; outside the source is refused.
    assert!(super::events(&click, None).is_err());
    let outside = action_of(None, None, None, Some("1920,0"), None, None).unwrap();
    assert!(super::events(&outside, size).is_err());
    let keys = action_of(None, Some("ctrl_left+c"), None, None, None, None).unwrap();
    assert_eq!(
        super::events(&keys, None).unwrap(),
        [
            InputEvent::Key {
                hid_usage: 0xE0,
                down: true
            },
            InputEvent::Key {
                hid_usage: 0x06,
                down: true
            },
            InputEvent::Key {
                hid_usage: 0x06,
                down: false
            },
            InputEvent::Key {
                hid_usage: 0xE0,
                down: false
            },
        ]
    );
    let scroll = action_of(None, None, None, None, Some("0,-2"), None).unwrap();
    assert_eq!(
        super::events(&scroll, None).unwrap(),
        [InputEvent::Wheel { dx: 0, dy: -240 }]
    );
    let home = action_of(None, None, None, None, None, Some("home")).unwrap();
    assert_eq!(home, InputAction::Action(SystemAction::Home));
    assert!(action_of(None, Some("hyper"), None, None, None, None).is_err());
    assert!(action_of(None, None, None, None, None, Some("reboot")).is_err());
    assert!(action_of(None, None, Some("10"), None, None, None).is_err());
    assert!(action_of(None, None, None, None, None, None).is_err());
}
