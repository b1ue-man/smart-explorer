//! Input of a controlling viewer (decisions C3 „Eingabe-Batch“ and C5): the
//! events, their compact big-endian batch in the `INPUT` frame, the key table
//! in USB HID usages including the toolbar keys egui cannot deliver (NR N4),
//! and the tracker that releases every pressed key, button and touch when
//! control ends, is given back or the connection is lost.
use std::io;

use serde::{Deserialize, Serialize};

use super::pixel_convert::FrameSize;
use super::wire::{invalid, BeReader, MAX_CONTROL_BYTES};

/// Most events of one batch: a batch covers one frame interval, and even an
/// 8 kHz mouse produces 128 events in 16 ms.
pub(crate) const MAX_INPUT_EVENTS: usize = 256;
/// Longest text of one event; longer text travels as several events.
pub(crate) const MAX_TEXT_BYTES: usize = 4096;
/// `Key.hid_usage` of a Consumer page (0x0C) usage is this bit plus the
/// usage; without it the value is a Keyboard/Keypad page (0x07) usage.
pub(crate) const CONSUMER_PAGE: u16 = 0x8000;

const POINTER_ABS: u8 = 1;
const BUTTON: u8 = 2;
const WHEEL: u8 = 3;
const KEY: u8 = 4;
const TEXT: u8 = 5;
const TOUCH: u8 = 6;
const ACTION: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointerButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TouchPhase {
    Start,
    Move,
    End,
    Cancel,
}

/// Global actions of the Android accessibility service (C3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemAction {
    Back,
    Home,
    Recents,
    Notifications,
    QuickSettings,
    LockScreen,
    PowerDialog,
}

// Wire codes are the position in these tables plus one.
#[rustfmt::skip]
const BUTTONS: [PointerButton; 5] = [
    PointerButton::Left, PointerButton::Right, PointerButton::Middle, PointerButton::Back,
    PointerButton::Forward,
];
#[rustfmt::skip]
const PHASES: [TouchPhase; 4] =
    [TouchPhase::Start, TouchPhase::Move, TouchPhase::End, TouchPhase::Cancel];
#[rustfmt::skip]
const ACTIONS: [SystemAction; 7] = [
    SystemAction::Back, SystemAction::Home, SystemAction::Recents, SystemAction::Notifications,
    SystemAction::QuickSettings, SystemAction::LockScreen, SystemAction::PowerDialog,
];

/// Wire code `index + 1` of a value in one of the tables above.
fn code_of<T: PartialEq>(table: &[T], value: &T) -> u8 {
    table
        .iter()
        .position(|entry| entry == value)
        .map_or(0, |index| index as u8 + 1)
}

fn from_code<T: Copy>(table: &[T], code: u8) -> io::Result<T> {
    usize::from(code)
        .checked_sub(1)
        .and_then(|index| table.get(index).copied())
        .ok_or_else(|| invalid("unknown screen input code"))
}

/// One input event. Pointer and touch positions are 0–65535 across the
/// source; the wheel counts 1/120 notches.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum InputEvent {
    PointerAbs {
        x: u16,
        y: u16,
    },
    Button {
        button: PointerButton,
        down: bool,
    },
    Wheel {
        dx: i16,
        dy: i16,
    },
    Key {
        hid_usage: u16,
        down: bool,
    },
    Text {
        text: String,
    },
    Touch {
        id: u8,
        phase: TouchPhase,
        x: u16,
        y: u16,
    },
    Action {
        action: SystemAction,
    },
}

/// What a host can inject, from `HELLO_OK`; the viewer offers only these.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InputCapabilities {
    #[serde(default)]
    pub pointer: bool,
    #[serde(default)]
    pub keyboard: bool,
    #[serde(default)]
    pub text: bool,
    #[serde(default)]
    pub touch: bool,
    #[serde(default)]
    pub actions: Vec<SystemAction>,
    /// The entries of `TOOLBAR_KEYS` the host can press.
    #[serde(default)]
    pub toolbar_keys: Vec<u16>,
}

pub(crate) const KEY_CAPS_LOCK: u16 = 0x39;
pub(crate) const KEY_PRINT_SCREEN: u16 = 0x46;
pub(crate) const KEY_SCROLL_LOCK: u16 = 0x47;
pub(crate) const KEY_PAUSE: u16 = 0x48;
pub(crate) const KEY_NUM_LOCK: u16 = 0x53;
pub(crate) const KEY_MENU: u16 = 0x65;
pub(crate) const KEY_SUPER_LEFT: u16 = 0xE3;
pub(crate) const KEY_PLAY_PAUSE: u16 = CONSUMER_PAGE | 0xCD;
pub(crate) const KEY_MUTE: u16 = CONSUMER_PAGE | 0xE2;
pub(crate) const KEY_VOLUME_UP: u16 = CONSUMER_PAGE | 0xE9;
pub(crate) const KEY_VOLUME_DOWN: u16 = CONSUMER_PAGE | 0xEA;

/// Keys without an egui event, offered as toolbar buttons (C5, NR N4).
#[rustfmt::skip]
pub(crate) const TOOLBAR_KEYS: [u16; 11] = [
    KEY_SUPER_LEFT, KEY_MENU, KEY_PRINT_SCREEN, KEY_PAUSE, KEY_SCROLL_LOCK, KEY_CAPS_LOCK,
    KEY_NUM_LOCK, KEY_PLAY_PAUSE, KEY_VOLUME_UP, KEY_VOLUME_DOWN, KEY_MUTE,
];

/// Named keys besides letters `a`–`z`, digits `0`–`9` and `f1`–`f24`.
#[rustfmt::skip]
const NAMED_KEYS: &[(&str, u16)] = &[
    ("enter", 0x28), ("escape", 0x29), ("backspace", 0x2A), ("tab", 0x2B), ("space", 0x2C),
    ("minus", 0x2D), ("equal", 0x2E), ("bracket_left", 0x2F), ("bracket_right", 0x30),
    ("backslash", 0x31), ("semicolon", 0x33), ("quote", 0x34), ("grave", 0x35),
    ("comma", 0x36), ("period", 0x37), ("slash", 0x38), ("caps_lock", KEY_CAPS_LOCK),
    ("print_screen", KEY_PRINT_SCREEN), ("scroll_lock", KEY_SCROLL_LOCK), ("pause", KEY_PAUSE),
    ("insert", 0x49), ("home", 0x4A), ("page_up", 0x4B), ("delete", 0x4C), ("end", 0x4D),
    ("page_down", 0x4E), ("right", 0x4F), ("left", 0x50), ("down", 0x51), ("up", 0x52),
    ("num_lock", KEY_NUM_LOCK), ("kp_divide", 0x54), ("kp_multiply", 0x55),
    ("kp_subtract", 0x56), ("kp_add", 0x57), ("kp_enter", 0x58), ("kp_1", 0x59),
    ("kp_2", 0x5A), ("kp_3", 0x5B), ("kp_4", 0x5C), ("kp_5", 0x5D), ("kp_6", 0x5E),
    ("kp_7", 0x5F), ("kp_8", 0x60), ("kp_9", 0x61), ("kp_0", 0x62), ("kp_decimal", 0x63),
    ("intl_backslash", 0x64), ("menu", KEY_MENU), ("cut", 0x7B), ("copy", 0x7C),
    ("paste", 0x7D), ("ctrl_left", 0xE0), ("shift_left", 0xE1), ("alt_left", 0xE2),
    ("super_left", KEY_SUPER_LEFT), ("ctrl_right", 0xE4), ("shift_right", 0xE5),
    ("alt_right", 0xE6), ("super_right", 0xE7), ("play_pause", KEY_PLAY_PAUSE),
    ("mute", KEY_MUTE), ("volume_up", KEY_VOLUME_UP), ("volume_down", KEY_VOLUME_DOWN),
    ("browser_back", CONSUMER_PAGE | 0x224),
];

/// A usage a host may receive: Keyboard page 0x04–0xE7 or Consumer page
/// 0x001–0x3FF.
pub(crate) fn valid_key_usage(usage: u16) -> bool {
    (0x04..=0xE7).contains(&usage) || (CONSUMER_PAGE + 1..=CONSUMER_PAGE + 0x3FF).contains(&usage)
}

/// The usage of a key name of `se screen input --key` (lower case).
pub(crate) fn key_usage(name: &str) -> Option<u16> {
    let bytes = name.as_bytes();
    if let [letter @ b'a'..=b'z'] = bytes {
        return Some(0x04 + u16::from(letter - b'a'));
    }
    if let [digit @ b'0'..=b'9'] = bytes {
        return Some(if *digit == b'0' {
            0x27
        } else {
            0x1E + u16::from(digit - b'1')
        });
    }
    if let Some(number) = name
        .strip_prefix('f')
        .and_then(|rest| rest.parse::<u16>().ok())
    {
        return match number {
            1..=12 => Some(0x3A + number - 1),
            13..=24 => Some(0x68 + number - 13),
            _ => None,
        };
    }
    NAMED_KEYS
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, usage)| *usage)
}

/// The name of a usage, the inverse of `key_usage`.
pub(crate) fn key_name(usage: u16) -> Option<String> {
    match usage {
        0x04..=0x1D => Some(char::from(b'a' + (usage - 0x04) as u8).to_string()),
        0x1E..=0x26 => Some(char::from(b'1' + (usage - 0x1E) as u8).to_string()),
        0x27 => Some("0".into()),
        0x3A..=0x45 => Some(format!("f{}", usage - 0x3A + 1)),
        0x68..=0x73 => Some(format!("f{}", usage - 0x68 + 13)),
        _ => NAMED_KEYS
            .iter()
            .find(|(_, value)| *value == usage)
            .map(|(name, _)| (*name).to_string()),
    }
}

/// The pixel of the source a position 0–65535 points at.
pub(crate) fn pointer_to_pixel(x: u16, y: u16, size: FrameSize) -> (u32, u32) {
    let axis = |value: u16, side: u32| {
        ((u64::from(value) * u64::from(side.saturating_sub(1)) + 32_767) / 65_535) as u32
    };
    (axis(x, size.width), axis(y, size.height))
}

/// The position 0–65535 of a pixel of the source; pixels beyond the source
/// map onto its edge.
pub(crate) fn pixel_to_pointer(px: u32, py: u32, size: FrameSize) -> (u16, u16) {
    let axis = |value: u32, side: u32| {
        let last = u64::from(side.saturating_sub(1));
        if last == 0 {
            0
        } else {
            ((u64::from(value).min(last) * 65_535 + last / 2) / last) as u16
        }
    };
    (axis(px, size.width), axis(py, size.height))
}

fn validate_event(event: &InputEvent) -> io::Result<()> {
    match event {
        InputEvent::Key { hid_usage, .. } if !valid_key_usage(*hid_usage) => {
            Err(invalid("screen input names an unknown key usage"))
        }
        InputEvent::Text { text }
            if text.is_empty() || text.len() > MAX_TEXT_BYTES || text.contains('\0') =>
        {
            Err(invalid(
                "screen input text must hold 1 to 4096 bytes without NUL",
            ))
        }
        _ => Ok(()),
    }
}

/// The payload of an `INPUT` frame: count `u16`, then per event a kind byte
/// and its fields, all big-endian.
pub(crate) fn encode_batch(events: &[InputEvent]) -> io::Result<Vec<u8>> {
    if events.is_empty() || events.len() > MAX_INPUT_EVENTS {
        return Err(invalid("screen input batch must hold 1 to 256 events"));
    }
    let mut out = Vec::with_capacity(2 + events.len() * 7);
    out.extend_from_slice(&(events.len() as u16).to_be_bytes());
    for event in events {
        validate_event(event)?;
        match event {
            InputEvent::PointerAbs { x, y } => {
                out.push(POINTER_ABS);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            InputEvent::Button { button, down } => {
                out.extend_from_slice(&[BUTTON, code_of(&BUTTONS, button), u8::from(*down)]);
            }
            InputEvent::Wheel { dx, dy } => {
                out.push(WHEEL);
                out.extend_from_slice(&dx.to_be_bytes());
                out.extend_from_slice(&dy.to_be_bytes());
            }
            InputEvent::Key { hid_usage, down } => {
                out.push(KEY);
                out.extend_from_slice(&hid_usage.to_be_bytes());
                out.push(u8::from(*down));
            }
            InputEvent::Text { text } => {
                out.push(TEXT);
                out.extend_from_slice(&(text.len() as u16).to_be_bytes());
                out.extend_from_slice(text.as_bytes());
            }
            InputEvent::Touch { id, phase, x, y } => {
                out.extend_from_slice(&[TOUCH, *id, code_of(&PHASES, phase)]);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            InputEvent::Action { action } => {
                out.extend_from_slice(&[ACTION, code_of(&ACTIONS, action)]);
            }
        }
    }
    if out.len() > MAX_CONTROL_BYTES {
        return Err(invalid("screen input batch exceeds 64 KiB"));
    }
    Ok(out)
}

fn flag(value: u8) -> io::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("screen input flag must be 0 or 1")),
    }
}

fn decode_event(reader: &mut BeReader) -> io::Result<InputEvent> {
    let event = match reader.u8()? {
        POINTER_ABS => InputEvent::PointerAbs {
            x: reader.u16()?,
            y: reader.u16()?,
        },
        BUTTON => InputEvent::Button {
            button: from_code(&BUTTONS, reader.u8()?)?,
            down: flag(reader.u8()?)?,
        },
        WHEEL => InputEvent::Wheel {
            dx: reader.i16()?,
            dy: reader.i16()?,
        },
        KEY => InputEvent::Key {
            hid_usage: reader.u16()?,
            down: flag(reader.u8()?)?,
        },
        TEXT => {
            let len = usize::from(reader.u16()?);
            if len > MAX_TEXT_BYTES {
                return Err(invalid("screen input text exceeds 4096 bytes"));
            }
            let text = std::str::from_utf8(reader.bytes(len)?)
                .map_err(|_| invalid("screen input text is not UTF-8"))?;
            InputEvent::Text { text: text.into() }
        }
        TOUCH => InputEvent::Touch {
            id: reader.u8()?,
            phase: from_code(&PHASES, reader.u8()?)?,
            x: reader.u16()?,
            y: reader.u16()?,
        },
        ACTION => InputEvent::Action {
            action: from_code(&ACTIONS, reader.u8()?)?,
        },
        _ => return Err(invalid("unknown screen input event")),
    };
    validate_event(&event)?;
    Ok(event)
}

pub(crate) fn decode_batch(payload: &[u8]) -> io::Result<Vec<InputEvent>> {
    let mut reader = BeReader::new(payload);
    let count = usize::from(reader.u16()?);
    if count == 0 || count > MAX_INPUT_EVENTS {
        return Err(invalid("screen input batch must hold 1 to 256 events"));
    }
    let mut events = Vec::with_capacity(count);
    for _ in 0..count {
        events.push(decode_event(&mut reader)?);
    }
    reader.finish()?;
    Ok(events)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Key(u16),
    Button(PointerButton),
    Touch { id: u8, x: u16, y: u16 },
}

/// Keys, buttons and touches the host has pressed for the controlling
/// viewer, in press order. The set is bounded by the value ranges (at most
/// 228 + 1023 keys, 5 buttons, 256 touches).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PressedInputs {
    held: Vec<Held>,
}

impl PressedInputs {
    /// Tracks one event the host applied.
    pub(crate) fn observe(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Key { hid_usage, down } => self.set(Held::Key(*hid_usage), *down),
            InputEvent::Button { button, down } => self.set(Held::Button(*button), *down),
            InputEvent::Touch { id, phase, x, y } => {
                self.held
                    .retain(|held| !matches!(held, Held::Touch { id: other, .. } if other == id));
                if matches!(phase, TouchPhase::Start | TouchPhase::Move) {
                    self.held.push(Held::Touch {
                        id: *id,
                        x: *x,
                        y: *y,
                    });
                }
            }
            InputEvent::PointerAbs { .. }
            | InputEvent::Wheel { .. }
            | InputEvent::Text { .. }
            | InputEvent::Action { .. } => {}
        }
    }

    fn set(&mut self, held: Held, down: bool) {
        let present = self.held.contains(&held);
        if down && !present {
            self.held.push(held);
        } else if !down && present {
            self.held.retain(|entry| *entry != held);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The events that release everything still pressed, newest first, so
    /// modifiers go up last; afterwards nothing is held.
    pub(crate) fn release_all(&mut self) -> Vec<InputEvent> {
        self.held
            .drain(..)
            .rev()
            .map(|held| match held {
                Held::Key(hid_usage) => InputEvent::Key {
                    hid_usage,
                    down: false,
                },
                Held::Button(button) => InputEvent::Button {
                    button,
                    down: false,
                },
                Held::Touch { id, x, y } => InputEvent::Touch {
                    id,
                    phase: TouchPhase::Cancel,
                    x,
                    y,
                },
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "input_model_tests.rs"]
mod tests;
