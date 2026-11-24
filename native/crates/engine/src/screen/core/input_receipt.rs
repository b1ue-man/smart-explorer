//! Input receipts (`input_receipts_v1`, round 12 rest, Codex 12/15): a
//! viewer that has to know what became of each input (`se screen input`, the
//! MCP tool `screen_input`) asks for them with `INPUT_RECEIPTS` after a host
//! that announced them in `HELLO_OK.input_receipts`; the host then answers
//! every `INPUT` it reads from then on, counted from 1, once it is settled:
//! applied (on an Android host: performed by the app), blocked, held back,
//! refused by the device with its sentence, or stopped because the control
//! was given up or the session ended before all of its text was typed.
//! The answer rides in `CONTROL_STATE.input` (a field older viewers skip),
//! so a viewer never waits a fixed time for a refusal that may still come.
//! A viewer that did not ask hears nothing new, except the stopped text
//! (`ERROR` `input_stopped`, `stopped_text`). Round 13 (S2): a text typed in
//! calls (`input_calls.rs`) whose later call failed is stopped too, with the
//! block or hold that stopped it (`block`, `held`; in the `ERROR` detail
//! `;block=<key>` or `;held=<key>`), not reported as held back or blocked
//! whole while most of it was typed.
use serde::{Deserialize, Serialize};

use super::wire_control::{ControlBlock, InputHeld};

/// The version a host announces in `HELLO_OK.input_receipts`.
pub const INPUT_RECEIPTS_V1: u16 = 1;
/// Tag of `INPUT_RECEIPTS` (viewer to host, one byte: on or off).
pub const INPUT_RECEIPTS: u8 = 57;
/// The `ERROR` code that tells a viewer without receipts that the rest of a
/// text was not typed; its detail is `<characters>` or `<characters>+`
/// (at most that many: the last call may still have typed), followed by
/// `;block=<key>` or `;held=<key>` when a block or hold stopped it partway
/// (round 13, S2).
pub const INPUT_STOPPED: &str = "input_stopped";

/// What became of the viewer's `batch`-th `INPUT` since it asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputReceipt {
    pub batch: u64,
    pub result: InputResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputResult {
    /// The host's platform took the whole batch (an Android host: its app
    /// performed it).
    Applied,
    /// Not applied; the block as `CONTROL_STATE.blocked` names it.
    Blocked { block: ControlBlock },
    /// Held back without applying it, as `CONTROL_STATE.held` names it.
    Held { held: InputHeld },
    /// The device took the batch but did not perform it; its sentence
    /// (`codec_reason_text::input_refusal_sentence`).
    Refused { reason: String },
    /// The control was given up or the session ended while the batch went
    /// in: `untyped` characters of its text were not typed (`exact: false`:
    /// at most that many, the running call may still have typed some).
    /// Round 13 (S2): with `block` or `held`, a call of its text failed
    /// after earlier calls went in, for that reason (fields an older viewer
    /// skips; absent when the control went or the session ended).
    Stopped {
        untyped: u64,
        exact: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        block: Option<ControlBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        held: Option<InputHeld>,
    },
    /// A result of a newer host.
    #[serde(other)]
    Other,
}

/// Characters of text in `events`.
pub fn text_characters<'a>(
    events: impl IntoIterator<Item = &'a super::input_model::InputEvent>,
) -> u64 {
    events
        .into_iter()
        .map(|event| match event {
            super::input_model::InputEvent::Text { text } => {
                u64::try_from(text.chars().count()).unwrap_or(u64::MAX)
            }
            _ => 0,
        })
        .fold(0, u64::saturating_add)
}

/// The German sentence for a text whose rest was not typed (a batch without
/// text: that it may not have been performed).
pub fn stopped_text(untyped: u64, exact: bool) -> String {
    if untyped == 0 {
        return "Die Steuerung wurde abgegeben oder die Sitzung beendet, bevor die Eingabe \
                sicher ausgeführt war."
            .into();
    }
    let count = if exact {
        format!("{untyped}")
    } else {
        format!("bis zu {untyped}")
    };
    format!(
        "Die Steuerung wurde abgegeben oder die Sitzung beendet, bevor der Text fertig getippt \
         war: {count} Zeichen wurden nicht getippt."
    )
}

/// Why a text stopped partway (round 13, S2), in German.
fn cause_text(block: Option<ControlBlock>, held: Option<InputHeld>) -> Option<&'static str> {
    if let Some(block) = block {
        return Some(match block {
            ControlBlock::ElevatedWindow => {
                "ein Fenster mit erhöhten Rechten nimmt keine Eingabe an"
            }
            ControlBlock::SecureDesktop => {
                "der gesicherte Desktop (Anmeldung, Sperre oder Benutzerkontensteuerung) ist aktiv"
            }
            ControlBlock::ControlUnavailable => "das Gerät hat die Eingabe nicht angenommen",
            ControlBlock::UnsupportedOnHost => "das Gerät unterstützt diese Eingabe nicht",
            ControlBlock::Other => "das Gerät hat die Eingabe abgelehnt",
        });
    }
    Some(match held? {
        InputHeld::ViewerHidden => "das Bild ist verdeckt oder minimiert",
        InputHeld::WaitingForPicture => "das Bild hat sich geändert und ist noch nicht angezeigt",
        InputHeld::ReleasePending => "gehaltene Tasten werden erst gelöst",
        InputHeld::BacklogFull => "zu viele Eingaben warten noch auf das Gerät",
        InputHeld::Other => "das Gerät hat die Eingabe zurückgehalten",
    })
}

/// The German sentence of a stopped batch: `stopped_text` when the control
/// went or the session ended; with the `block` or `held` that stopped its
/// text partway (round 13, S2), that reason.
pub fn stopped_result_text(
    untyped: u64,
    exact: bool,
    block: Option<ControlBlock>,
    held: Option<InputHeld>,
) -> String {
    let Some(cause) = cause_text(block, held) else {
        return stopped_text(untyped, exact);
    };
    let count = if exact {
        format!("{untyped}")
    } else {
        format!("bis zu {untyped}")
    };
    format!(
        "Der Text wurde nicht fertig getippt, weil {cause}: {count} Zeichen wurden nicht getippt."
    )
}

/// `stopped_result_text` from the detail of an `ERROR` `input_stopped`.
pub fn stopped_detail_text(detail: Option<&str>) -> Option<String> {
    let (detail, cause) = match detail?.trim().split_once(';') {
        Some((number, cause)) => (number.trim(), Some(cause.trim())),
        None => (detail?.trim(), None),
    };
    let (number, exact) = match detail.strip_suffix('+') {
        Some(number) => (number, false),
        None => (detail, true),
    };
    let untyped = number.parse::<u64>().ok()?;
    let key = |prefix: &str| {
        cause
            .and_then(|cause| cause.strip_prefix(prefix))
            .map(|key| serde_json::Value::String(key.into()))
    };
    let block = key("block=").and_then(|key| serde_json::from_value(key).ok());
    let held = key("held=").and_then(|key| serde_json::from_value(key).ok());
    Some(stopped_result_text(untyped, exact, block, held))
}

/// The detail of an `ERROR` `input_stopped`.
pub fn stopped_detail(untyped: u64, exact: bool) -> String {
    if exact {
        format!("{untyped}")
    } else {
        format!("{untyped}+")
    }
}

/// The detail of an `ERROR` `input_stopped` for a text a `block` or `held`
/// stopped partway (round 13, S2): `stopped_detail` with `;block=<key>` or
/// `;held=<key>`.
pub fn stopped_cause_detail(
    untyped: u64,
    exact: bool,
    block: Option<ControlBlock>,
    held: Option<InputHeld>,
) -> String {
    let key = |value: serde_json::Result<serde_json::Value>| {
        value
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
    };
    let cause = match (block, held) {
        (Some(block), _) => key(serde_json::to_value(block)).map(|key| format!(";block={key}")),
        (None, Some(held)) => key(serde_json::to_value(held)).map(|key| format!(";held={key}")),
        (None, None) => None,
    };
    format!(
        "{}{}",
        stopped_detail(untyped, exact),
        cause.unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::input_model::InputEvent;
    use crate::screen::core::wire_control::ControlState;

    /// Round 12 rest (Codex 12/15): a receipt rides in `CONTROL_STATE.input`,
    /// an older viewer reads the state without it, a result of a newer host
    /// reads as `Other`; the stopped text names the untyped characters in
    /// German, also from the `ERROR` detail of a viewer without receipts.
    #[test]
    fn screen_task_input_receipt_wire_and_stopped_text() {
        let state = ControlState {
            holder: None,
            blocked: None,
            held: None,
            input: Some(InputReceipt {
                batch: 3,
                result: InputResult::Stopped {
                    untyped: 37,
                    exact: true,
                    block: None,
                    held: None,
                },
            }),
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(
            json["input"],
            serde_json::json!({"batch": 3, "result": {"kind": "stopped", "untyped": 37, "exact": true}})
        );
        let back: ControlState = serde_json::from_value(json).unwrap();
        assert_eq!(back, state);
        let refused: InputReceipt = serde_json::from_value(serde_json::json!({
            "batch": 1, "result": {"kind": "refused", "reason": "the device did not answer an input in time"}
        }))
        .unwrap();
        assert!(matches!(refused.result, InputResult::Refused { .. }));
        let newer: InputReceipt = serde_json::from_value(serde_json::json!({
            "batch": 2, "result": {"kind": "queued_elsewhere"}
        }))
        .unwrap();
        assert_eq!(newer.result, InputResult::Other);
        // An older viewer's state type (no `input`) reads the frame.
        #[derive(serde::Deserialize)]
        struct Older {
            #[allow(dead_code)]
            holder: Option<crate::screen::core::wire_control::ControlHolder>,
        }
        let older: Result<Older, _> = serde_json::to_value(&state).and_then(serde_json::from_value);
        assert!(older.is_ok());

        let events = [
            InputEvent::Text { text: "äb".into() },
            InputEvent::Key {
                hid_usage: 4,
                down: true,
            },
            InputEvent::Text { text: "c".into() },
        ];
        assert_eq!(text_characters(&events), 3);
        assert!(stopped_text(37, true).contains(": 37 Zeichen wurden nicht getippt."));
        assert!(stopped_text(5, false).contains("bis zu 5 Zeichen"));
        assert!(stopped_text(0, false).ends_with("bevor die Eingabe sicher ausgeführt war."));
        assert_eq!(stopped_detail(5, false), "5+");
        assert_eq!(
            stopped_detail_text(Some("5+")),
            Some(stopped_text(5, false))
        );
        assert_eq!(
            stopped_detail_text(Some("12")),
            Some(stopped_text(12, true))
        );
        assert_eq!(stopped_detail_text(Some("x")), None);

        // Round 13 (S2): a text stopped partway names why, on the wire (an
        // older viewer skips the field) and in the `ERROR` detail.
        let partway = InputResult::Stopped {
            untyped: 120,
            exact: false,
            block: Some(ControlBlock::SecureDesktop),
            held: None,
        };
        let json = serde_json::to_value(&partway).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "stopped", "untyped": 120, "exact": false, "block": "secure_desktop"})
        );
        assert_eq!(
            serde_json::from_value::<InputResult>(json).unwrap(),
            partway
        );
        let text = stopped_result_text(120, false, Some(ControlBlock::SecureDesktop), None);
        assert!(
            text.starts_with("Der Text wurde nicht fertig getippt, weil der gesicherte Desktop")
        );
        assert!(text.ends_with(": bis zu 120 Zeichen wurden nicht getippt."));
        assert_eq!(
            stopped_result_text(3, true, None, None),
            stopped_text(3, true)
        );
        let detail = stopped_cause_detail(120, false, Some(ControlBlock::SecureDesktop), None);
        assert_eq!(detail, "120+;block=secure_desktop");
        assert_eq!(stopped_detail_text(Some(&detail)), Some(text));
        let held = stopped_cause_detail(7, true, None, Some(InputHeld::WaitingForPicture));
        assert_eq!(held, "7;held=waiting_for_picture");
        assert_eq!(
            stopped_detail_text(Some(&held)),
            Some(stopped_result_text(
                7,
                true,
                None,
                Some(InputHeld::WaitingForPicture)
            ))
        );
        assert_eq!(stopped_cause_detail(4, true, None, None), "4");
    }
}
