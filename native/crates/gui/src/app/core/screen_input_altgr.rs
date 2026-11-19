//! AltGr characters and swallowed paste shortcuts of the viewer's input
//! (Gemini 5/9). A Windows viewer reports AltGr as Ctrl+Alt and egui-winit
//! then drops the character (egui-winit 0.29.1 `lib.rs` 794: no `Text`
//! under Ctrl), so `@` (AltGr+Q, German layout) went out as Ctrl+Alt+Q: a
//! shortcut on a Linux host, nothing on an Android one. X11 and Wayland
//! report AltGr as level 3, neither Alt nor Ctrl (winit 0.30 `xkb/state.rs`
//! 129: Alt is `Mod1` only), so egui hands the `Text` over there and this
//! stays off. egui-winit also turns Ctrl+V and (Windows) Umschalt+Einfg
//! into `Paste` with the viewer's clipboard and sends nothing at all when
//! that holds no text (`lib.rs` 766-774).
use std::time::{Duration, Instant};

use super::*;

#[cfg(windows)]
#[path = "../os/windows/screen_keyboard.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "../os/linux_os/screen_keyboard.rs"]
mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use eframe::egui::Key;

    pub fn altgr_held() -> bool {
        false
    }

    pub fn altgr_text(_physical: Key, _shift: bool) -> Option<String> {
        None
    }

    pub fn paste_keys_held(_ctrl: bool, _shift: bool) -> bool {
        false
    }
}

/// The character a logical key of egui stands for (egui-winit names a
/// layout's character so where egui has a key for it).
fn logical_char(key: Key) -> Option<char> {
    Some(match key {
        Key::Backslash => '\\',
        Key::Pipe => '|',
        Key::OpenBracket => '[',
        Key::CloseBracket => ']',
        Key::Colon => ':',
        Key::Comma => ',',
        Key::Minus => '-',
        Key::Period => '.',
        Key::Plus => '+',
        Key::Equals => '=',
        Key::Semicolon => ';',
        Key::Slash => '/',
        Key::Questionmark => '?',
        Key::Backtick => '`',
        Key::Quote => '\'',
        _ => return None,
    })
}

/// How long a swallowed paste is waited for (`missed_paste`).
const PASTE_WAIT: Duration = Duration::from_millis(50);

/// One press of Ctrl+V or Umschalt+Einfg (`missed_paste`).
struct Press {
    since: Instant,
    /// Frames since the one that saw the keys down.
    frames: u32,
    /// Umschalt+Einfg rather than Ctrl+V.
    insert: bool,
    /// Decided: egui's `Paste` came, or the shortcut went out.
    done: bool,
}

/// The paste watch of a mapper.
#[derive(Default)]
pub(super) struct PasteWatch {
    press: Option<Press>,
    last_paste: Option<Instant>,
}

impl InputMapper {
    /// Ctrl and Alt are AltGr for a host that takes text: they are not sent.
    fn altgr(&self, mods: Modifiers) -> bool {
        self.caps.text && mods.ctrl && mods.alt && platform::altgr_held()
    }

    /// Sends the modifier keys that changed, without Ctrl and Alt while they
    /// are AltGr (`altgr_key` sends its character as text).
    pub(super) fn sync_modifiers(&mut self, now: Modifiers) {
        let now = if self.altgr(now) {
            Modifiers {
                ctrl: false,
                alt: false,
                ..now
            }
        } else {
            now
        };
        self.sync_exact(now);
    }

    /// A key pressed with AltGr: the character it types goes out as `Text`
    /// without Ctrl and Alt (the host's own layout cannot know it); a key
    /// that types none goes out as itself with Ctrl and Alt, so a shortcut
    /// stays one. `false`: no AltGr press, the caller maps it as before.
    pub(super) fn altgr_key(
        &mut self,
        key: Key,
        physical: Option<Key>,
        pressed: bool,
        repeat: bool,
        modifiers: Modifiers,
    ) -> bool {
        if !pressed || !self.altgr(modifiers) {
            return false;
        }
        // egui names `\`, `|`, `[` and `]` as the logical key itself, its
        // place as `physical_key` (none for the key beside left Umschalt).
        let named = logical_char(key)
            .filter(|_| physical != Some(key))
            .map(String::from);
        let text = named.or_else(|| platform::altgr_text(physical?, modifiers.shift));
        let Some(text) = text else {
            self.sync_exact(modifiers);
            self.key(key, pressed, repeat, modifiers);
            return true;
        };
        // Its release is not sent either (`key`, `text_keys`).
        if !self.text_keys.contains(&key) {
            self.text_keys.push(key);
        }
        self.text(&text);
        true
    }

    /// egui-winit reads AltGr+C, X or V as Ctrl+C, X or V (`lib.rs` 758-775,
    /// 1023-1039: `command` is Ctrl) and makes `Copy`, `Cut` or `Paste` of
    /// it, never a key (round 8: Polish AltGr+C is „ć“, Hungarian AltGr+V
    /// „@“). With AltGr held the character of `physical` goes out as `Text`;
    /// `false` (none, or no AltGr): the shortcut as before.
    pub(super) fn altgr_clipboard(&mut self, physical: Key) -> bool {
        if !self.caps.text || !platform::altgr_held() {
            return false;
        }
        let Some(text) = platform::altgr_text(physical, self.mods.shift) else {
            return false;
        };
        self.text(&text);
        true
    }

    /// Ctrl+V or Umschalt+Einfg that egui-winit swallowed (a clipboard
    /// without text): the shortcut goes to the host once per press, which
    /// pastes its own clipboard. egui-winit makes `Paste` on the press only
    /// (`lib.rs` 760-775, inside `if pressed`), so the press is decided
    /// `PASTE_WAIT` and two frames after the keys were seen down, or when they
    /// are let go: a `Paste` in any frame of that wait (or `PASTE_WAIT` before
    /// it) means egui had the text, and nothing goes out twice whatever
    /// frame the key state and the event fall into. `frame_mods` are
    /// `NONE` while a toolbar field holds the keyboard.
    pub(super) fn missed_paste(&mut self, events: &[Event], mods: Modifiers) {
        let now = Instant::now();
        if events.iter().any(|event| matches!(event, Event::Paste(_))) {
            self.paste.last_paste = Some(now);
        }
        let recent =
            |at: Option<Instant>, since: Instant| at.is_some_and(|at| at + PASTE_WAIT >= since);
        // AltGr+V is a character on some layouts (`altgr_clipboard`).
        let altgr = self.caps.text && platform::altgr_held();
        let held = !altgr && platform::paste_keys_held(mods.ctrl, mods.shift);
        if held && self.paste.press.is_none() {
            self.paste.press = Some(Press {
                since: now,
                frames: 0,
                insert: mods.shift && !mods.ctrl,
                done: false,
            });
        } else if let Some(press) = self.paste.press.as_mut() {
            press.frames += 1;
        }
        let last = self.paste.last_paste;
        let Some(press) = self.paste.press.as_mut() else {
            return;
        };
        press.done |= recent(last, press.since);
        let due = !held || (press.frames >= 2 && now >= press.since + PASTE_WAIT);
        if due && !press.done {
            press.done = true;
            if press.insert {
                let shifted = Modifiers {
                    shift: true,
                    ctrl: false,
                    ..self.mods
                };
                self.sync_modifiers(shifted);
                self.tap("insert");
            } else {
                self.chord("v");
            }
        }
        if !held {
            self.paste.press = None;
        }
    }
}
