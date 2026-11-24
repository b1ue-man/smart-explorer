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
    // Every pixel comes back as itself where the host rounds down (Codex 6/18).
    for pixel in [0u32, 1, 2, 959, 1918, 1919] {
        let text = format!("{pixel},0");
        let click = action_of(None, None, Some(&text), None, None, None).unwrap();
        match super::events(&click, size).unwrap()[0] {
            InputEvent::PointerAbs { x, .. } => {
                assert_eq!(u32::from(x) * 1919 / 65_535, pixel, "{pixel}")
            }
            ref other => panic!("{other:?}"),
        }
    }
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

/// Round 12, S9 / Codex 12/15: a host without input receipts (an older
/// version) answers a refused input after its strokes and the wait for its
/// accessibility service; the command waits that long before it says `sent`
/// (unconfirmed), a desktop host as before.
#[test]
fn screen_task_cli_input_android_refusal_is_waited_for() {
    assert_eq!(answer_wait(HostPlatform::Windows), BLOCK_WAIT);
    assert_eq!(answer_wait(HostPlatform::Linux), BLOCK_WAIT);
    // `AccessLink.INPUT_CONNECT_WAIT_MS` once, two scroll strokes of 150 ms
    // with `ScreenHostControl.STROKE_GRACE_MS` each, then the way back.
    assert_eq!(
        answer_wait(HostPlatform::Android),
        BLOCK_WAIT + Duration::from_millis(1000 + 2 * (150 + 500))
    );
}

/// Round 12 rest (Codex 12/15): with a host that sends input receipts the
/// command waits for the receipt of its input and reports what it says
/// (applied, refused with the device's sentence, stopped with the untyped
/// characters); states without a receipt do not end the wait. Without
/// receipts `sent` stays unconfirmed (`applied: null`). Round 13: the wait
/// ends after `receipt_wait`, derived from the host's limits for the batch,
/// with `applied: null` and the reason in `unconfirmed` (S4); a text stopped
/// partway names its block (S2).
#[test]
fn screen_task_cli_input_waits_for_the_receipt() {
    use super::answer::{outcome_line, receipt_outcome, unconfirmed_outcome};
    use crate::screen::core::input_receipt::{InputReceipt, InputResult};
    use crate::screen::core::wire_control::ControlState;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    let state = |held: Option<InputHeld>, result: Option<InputResult>| {
        StreamEvent::Host(HostFrame::ControlState(ControlState {
            holder: None,
            blocked: None,
            held,
            input: result.map(|result| InputReceipt { batch: 1, result }),
        }))
    };
    let answer = |events: Vec<StreamEvent>| {
        let events = RefCell::new(VecDeque::from(events));
        let next = |_: Option<Duration>| -> Result<Option<StreamEvent>, String> {
            Ok(events.borrow_mut().pop_front())
        };
        receipt_outcome(&next, Duration::from_secs(60))
    };
    let sentence = "the device did not answer an input in time";
    let refused = answer(vec![
        state(Some(InputHeld::ReleasePending), None),
        state(
            None,
            Some(InputResult::Refused {
                reason: sentence.into(),
            }),
        ),
    ])
    .unwrap();
    assert!(!refused.sent);
    assert_eq!(refused.applied, Some(false));
    assert_eq!(refused.refused.as_deref(), Some(sentence));
    let value = outcome_value(&refused);
    assert_eq!(outcome_line(&value), format!("refused\t{sentence}\n"));

    let applied = answer(vec![state(None, Some(InputResult::Applied))]).unwrap();
    assert!(applied.sent);
    assert_eq!(outcome_line(&outcome_value(&applied)), "applied\n");

    let stopped = answer(vec![state(
        None,
        Some(InputResult::Stopped {
            untyped: 5,
            exact: false,
            block: None,
            held: None,
        }),
    )])
    .unwrap();
    assert_eq!(
        stopped.stopped,
        Some(Stopped {
            untyped: 5,
            exact: false,
            block: None,
            held: None,
        })
    );
    let value = outcome_value(&stopped);
    assert_eq!(value["stopped"]["untyped"], 5);
    assert!(outcome_line(&value).contains("bis zu 5 Zeichen wurden nicht getippt"));
    let partway = answer(vec![state(
        None,
        Some(InputResult::Stopped {
            untyped: 70,
            exact: false,
            block: Some(ControlBlock::ElevatedWindow),
            held: None,
        }),
    )])
    .unwrap();
    let value = outcome_value(&partway);
    assert_eq!(value["stopped"]["block"], "elevated_window");
    assert!(outcome_line(&value).starts_with("stopped\tDer Text wurde nicht fertig getippt, weil"));

    // Round 13 (S4): no receipt within the wait: sent, `applied: null`, why.
    let lost = answer(vec![state(Some(InputHeld::ReleasePending), None)]).unwrap();
    assert!(lost.sent);
    assert_eq!(lost.applied, None);
    let value = outcome_value(&lost);
    assert!(value["applied"].is_null());
    assert!(value["unconfirmed"]
        .as_str()
        .is_some_and(|text| text.contains("no receipt")));
    assert!(outcome_line(&value).starts_with("sent\tthe device sent no receipt"));
    // The wait covers the host's limits for the batch: per call of text its
    // link wait and answer time, and the time per character.
    let short = [InputEvent::Text { text: "a".into() }];
    let base = answer_wait(HostPlatform::Linux) + ANSWER_WAIT;
    assert_eq!(
        receipt_wait(HostPlatform::Linux, &short),
        base + INPUT_TIMEOUT * 2 + TEXT_CHARACTER_TIME
    );
    let long = [InputEvent::Text {
        text: "x".repeat(TEXT_CALL_CHARACTERS * 3 + 1),
    }];
    let characters = u32::try_from(TEXT_CALL_CHARACTERS * 3 + 1).unwrap();
    assert_eq!(
        receipt_wait(HostPlatform::Linux, &long),
        base + INPUT_TIMEOUT * 8 + TEXT_CHARACTER_TIME * characters
    );

    // An older host: nothing within the wait, `sent` without confirmation.
    let silent = |_: Option<Duration>| -> Result<Option<StreamEvent>, String> { Ok(None) };
    let unconfirmed = unconfirmed_outcome(&silent, Duration::from_millis(1)).unwrap();
    assert!(unconfirmed.sent);
    let value = outcome_value(&unconfirmed);
    assert!(value["applied"].is_null());
    assert_eq!(outcome_line(&value), "sent\n");
}
