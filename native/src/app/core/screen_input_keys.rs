//! The key table of the desktop viewer (SCR-M7, U25-8): `egui::Key` to the
//! names of `input_model::key_usage` (HID usages), which keys carry text,
//! and the three shortcuts the viewer keeps for itself (U25-7). egui has no
//! event for the Windows key, Menu, Print, Pause, Scroll lock, Caps lock,
//! Num lock and the media keys: the toolbar sends them (`TOOLBAR_KEYS`).
use eframe::egui::{Event, Key, Modifiers};

use crate::screen::core::input_model::key_usage;

/// Keys the viewer keeps for itself (U25-7); everything else goes to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalKey {
    /// F11 / Ctrl+Alt+Shift+F (Ctrl+Alt+Pause comes from the OS adapter).
    Fullscreen,
    ExitFullscreen,
    /// Ctrl+Shift+S.
    Stats,
    /// Ctrl+Shift+K.
    ToggleControl,
}

pub(crate) fn local_key(key: Key, modifiers: Modifiers) -> Option<LocalKey> {
    match key {
        Key::F11 => Some(LocalKey::Fullscreen),
        Key::F if modifiers.ctrl && modifiers.alt && modifiers.shift => {
            Some(LocalKey::ExitFullscreen)
        }
        Key::S if modifiers.ctrl && modifiers.shift => Some(LocalKey::Stats),
        Key::K if modifiers.ctrl && modifiers.shift => Some(LocalKey::ToggleControl),
        _ => None,
    }
}

/// The viewer's own shortcuts pressed in `events`.
pub(crate) fn local_keys(events: &[Event]) -> Vec<LocalKey> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => local_key(*key, *modifiers),
            _ => None,
        })
        .collect()
}

macro_rules! key_names {
    ($key:expr; $($variant:ident => $name:expr),+ $(,)?) => {
        match $key {
            $(Key::$variant => Some($name),)+
            _ => None,
        }
    };
}

/// The name of `key` for `input_model::key_usage`; `None` for keys without
/// a usage (F25–F35, and what egui does not know).
pub(crate) fn key_name_of(key: Key) -> Option<&'static str> {
    key_names!(key;
        A => "a", B => "b", C => "c", D => "d", E => "e", F => "f", G => "g", H => "h",
        I => "i", J => "j", K => "k", L => "l", M => "m", N => "n", O => "o", P => "p",
        Q => "q", R => "r", S => "s", T => "t", U => "u", V => "v", W => "w", X => "x",
        Y => "y", Z => "z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4", Num5 => "5",
        Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
        F1 => "f1", F2 => "f2", F3 => "f3", F4 => "f4", F5 => "f5", F6 => "f6",
        F7 => "f7", F8 => "f8", F9 => "f9", F10 => "f10", F11 => "f11", F12 => "f12",
        F13 => "f13", F14 => "f14", F15 => "f15", F16 => "f16", F17 => "f17", F18 => "f18",
        F19 => "f19", F20 => "f20", F21 => "f21", F22 => "f22", F23 => "f23", F24 => "f24",
        ArrowUp => "up", ArrowDown => "down", ArrowLeft => "left", ArrowRight => "right",
        Enter => "enter", Escape => "escape", Tab => "tab", Backspace => "backspace",
        Space => "space", Insert => "insert", Delete => "delete", Home => "home", End => "end",
        PageUp => "page_up", PageDown => "page_down",
        Minus => "minus", Equals => "equal", Plus => "equal",
        OpenBracket => "bracket_left", CloseBracket => "bracket_right",
        Backslash => "backslash", Pipe => "backslash",
        Semicolon => "semicolon", Colon => "semicolon",
        Quote => "quote", Backtick => "grave", Comma => "comma", Period => "period",
        Slash => "slash", Questionmark => "slash",
        Copy => "copy", Cut => "cut", Paste => "paste",
    )
}

/// The HID usage of `key`.
pub(crate) fn key_usage_of(key: Key) -> Option<u16> {
    key_name_of(key).and_then(key_usage)
}

/// Keys whose press comes with a `Text` event when no Ctrl or Alt is held:
/// the host gets the character, not the key.
pub(crate) fn is_text_key(key: Key) -> bool {
    key_name_of(key).is_some_and(|name| name.len() == 1)
        || matches!(
            key,
            Key::Space
                | Key::Minus
                | Key::Equals
                | Key::Plus
                | Key::OpenBracket
                | Key::CloseBracket
                | Key::Backslash
                | Key::Pipe
                | Key::Semicolon
                | Key::Colon
                | Key::Quote
                | Key::Backtick
                | Key::Comma
                | Key::Period
                | Key::Slash
                | Key::Questionmark
        )
}

#[cfg(test)]
#[path = "batches_exit_task_tests.rs"]
mod batches_task_tests;
