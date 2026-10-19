//! Keys of the layer backend (DSK-M7, U37-11): the table of X11 keysyms the
//! surface reacts to and the translation of one key press into egui events.
//! Keys outside the table only produce text. Pure, so the suite checks it
//! without a compositor.
use eframe::egui::{Event, Key, Modifiers};

const LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];
const DIGITS: [Key; 10] = [
    Key::Num0,
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];
const FUNCTION: [Key; 12] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
];

/// The egui key of an X11 keysym (`XKB_KEY_*`): letters (also capitals),
/// digits of the main block and the keypad (egui knows only one digit
/// block), the cursor and editing keys, Enter (also the keypad's), Tab (also
/// the back tab of Shift+Tab), F1 to F12 and the punctuation keys.
pub(super) fn keysym_to_egui(sym: u32) -> Option<Key> {
    Some(match sym {
        0x61..=0x7a => LETTERS[(sym - 0x61) as usize],
        0x41..=0x5a => LETTERS[(sym - 0x41) as usize],
        0x30..=0x39 => DIGITS[(sym - 0x30) as usize],
        0xffb0..=0xffb9 => DIGITS[(sym - 0xffb0) as usize],
        0xffbe..=0xffc9 => FUNCTION[(sym - 0xffbe) as usize],
        0x20 => Key::Space,
        0xff09 | 0xfe20 => Key::Tab,
        0xff0d | 0xff8d => Key::Enter,
        0xff1b => Key::Escape,
        0xff08 => Key::Backspace,
        0xffff => Key::Delete,
        0xff63 => Key::Insert,
        0xff50 => Key::Home,
        0xff57 => Key::End,
        0xff55 => Key::PageUp,
        0xff56 => Key::PageDown,
        0xff51 => Key::ArrowLeft,
        0xff52 => Key::ArrowUp,
        0xff53 => Key::ArrowRight,
        0xff54 => Key::ArrowDown,
        0x2d => Key::Minus,
        0x2b => Key::Plus,
        0x3d => Key::Equals,
        0x2c => Key::Comma,
        0x2e => Key::Period,
        0x2f => Key::Slash,
        0x3b => Key::Semicolon,
        0x5c => Key::Backslash,
        0x5b => Key::OpenBracket,
        0x5d => Key::CloseBracket,
        0x27 => Key::Quote,
        0x60 => Key::Backtick,
        _ => return None,
    })
}

/// egui's modifiers from the three xkb modifiers; `command` is Ctrl.
pub(super) fn modifiers_of(alt: bool, ctrl: bool, shift: bool) -> Modifiers {
    Modifiers {
        alt,
        ctrl,
        shift,
        mac_cmd: false,
        command: ctrl,
    }
}

fn is_cut(modifiers: Modifiers, key: Key) -> bool {
    (modifiers.command && key == Key::X) || (modifiers.shift && key == Key::Delete)
}

fn is_copy(modifiers: Modifiers, key: Key) -> bool {
    (modifiers.command && key == Key::C) || (modifiers.ctrl && key == Key::Insert)
}

fn is_paste(modifiers: Modifiers, key: Key) -> bool {
    (modifiers.command && key == Key::V) || (modifiers.shift && key == Key::Insert)
}

/// Text of a key is only typed text when it is printable (no control
/// character, no private use area).
pub(super) fn is_typed_text(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| {
            let code = c as u32;
            !c.is_control()
                && !(0xe000..=0xf8ff).contains(&code)
                && !(0xf0000..=0xffffd).contains(&code)
                && !(0x100000..=0x10fffd).contains(&code)
        })
}

/// One key press or release as the keyboard reports it.
pub(super) struct KeyPress<'a> {
    pub(super) sym: u32,
    /// The text the key produces in the current layout (`xkb_state_key_get_utf8`).
    pub(super) text: Option<&'a str>,
    pub(super) pressed: bool,
    pub(super) repeat: bool,
    pub(super) modifiers: Modifiers,
}

/// The egui events of a key. Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+Insert, Shift+Insert
/// and Shift+Delete are not keys but `Copy`, `Cut` and `Paste` (as in
/// egui-winit); `paste` yields the text of the clipboard, empty when it holds
/// none or a file list (the view reads files itself). A repeat of such a key
/// does nothing. Text only without Ctrl and Alt.
pub(super) fn key_events(press: KeyPress<'_>, paste: impl FnOnce() -> String) -> Vec<Event> {
    let mut events = Vec::new();
    let KeyPress {
        sym,
        text,
        pressed,
        repeat,
        modifiers,
    } = press;
    if let Some(key) = keysym_to_egui(sym) {
        if pressed
            && (is_cut(modifiers, key) || is_copy(modifiers, key) || is_paste(modifiers, key))
        {
            if !repeat {
                events.push(if is_cut(modifiers, key) {
                    Event::Cut
                } else if is_copy(modifiers, key) {
                    Event::Copy
                } else {
                    Event::Paste(paste())
                });
            }
            return events;
        }
        events.push(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        });
    }
    if let Some(text) = text {
        let typed = pressed && !modifiers.ctrl && !modifiers.alt && !modifiers.command;
        if typed && is_typed_text(text) {
            events.push(Event::Text(text.to_string()));
        }
    }
    events
}
