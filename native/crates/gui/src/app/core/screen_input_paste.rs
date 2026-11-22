//! Ctrl+V and (Windows) Umschalt+Einfg that egui-winit swallowed (Gemini
//! 5/9): it turns them into `Paste` with the viewer's clipboard, on the
//! press only (egui-winit 0.29.1 `lib.rs` 760-775, inside `if pressed`),
//! and sends nothing at all when that clipboard holds no text, so the host
//! never pasted its own. Its release still arrives as a key (`lib.rs`
//! 777-783, with the modifiers of that moment). Without a `Paste` of the
//! press the shortcut goes to the host once: at the release (every
//! platform, Wayland included), or earlier where the platform tells which
//! keys are down (Windows `GetKeyState`, X11 `QueryKeymap`), `PASTE_WAIT`
//! and two frames after the keys were seen down. One flag per press
//! (`handled`: egui's `Paste` came, or the shortcut went out) keeps both
//! ways from sending it twice; the release ends the press.
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
    pub fn insert_held() -> bool {
        false
    }

    pub fn paste_keys_held(_ctrl: bool, _shift: bool) -> bool {
        false
    }

    pub fn ctrl_alt_text(_physical: super::Key, _shift: bool) -> Option<String> {
        None
    }
}

/// How long a swallowed paste is waited for before the key state decides.
const PASTE_WAIT: Duration = Duration::from_millis(50);

/// One press seen through the key state (`missed_paste`).
struct Press {
    since: Instant,
    /// Frames since the one that saw the keys down.
    frames: u32,
    /// Umschalt+Einfg rather than Ctrl+V.
    insert: bool,
}

/// The paste watch of a mapper.
#[derive(Default)]
pub(super) struct PasteWatch {
    press: Option<Press>,
    /// This press is decided: egui's `Paste` came, or the shortcut went out.
    handled: bool,
    /// The modifiers of this frame (Gemini 8/12: `Cut`/`Paste` arrive
    /// before the frame's modifiers are sent, `cut`/`paste` read them here).
    pub(super) mods: Modifiers,
    /// Keys typed as their Ctrl+left-Alt character (`key_event`): their
    /// release is no key for the host.
    typed: Vec<Key>,
}

impl InputMapper {
    /// `Copy`: Strg+Einfg as itself where egui-winit made it `Copy` (Codex
    /// 8/11: a terminal copies on Strg+Einfg and interrupts on Strg+C),
    /// else Strg+C.
    pub(super) fn copy(&mut self) {
        if !platform::insert_held() {
            self.chord("c");
            return;
        }
        let held = Modifiers {
            ctrl: true,
            ..self.mods
        };
        self.sync_modifiers(held);
        self.tap("insert");
    }

    /// Ctrl+V (`v`) or Umschalt+Einfg (`insert`) to the host, which pastes
    /// its own clipboard.
    fn paste_shortcut(&mut self, insert: bool) {
        self.paste.handled = true;
        if insert {
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

    /// At the start of each frame: notes egui's `Paste` and, where the key
    /// state is known, sends a swallowed shortcut before its release.
    /// `frame_mods` are `NONE` while a toolbar field holds the keyboard.
    pub(super) fn missed_paste(&mut self, events: &[Event], mods: Modifiers) {
        self.paste.mods = mods;
        if events.iter().any(|event| matches!(event, Event::Paste(_))) {
            self.paste.handled = true;
        }
        let held = platform::paste_keys_held(mods.ctrl, mods.shift);
        let now = Instant::now();
        match self.paste.press.as_mut() {
            None if held => {
                self.paste.press = Some(Press {
                    since: now,
                    frames: 0,
                    insert: mods.shift && !mods.ctrl,
                });
            }
            Some(press) => press.frames += 1,
            None => {}
        }
        let Some(press) = self.paste.press.as_ref() else {
            return;
        };
        let insert = press.insert;
        let due = !held || (press.frames >= 2 && now >= press.since + PASTE_WAIT);
        if !held {
            self.paste.press = None;
        }
        if due && !self.paste.handled {
            self.paste_shortcut(insert);
        }
    }

    /// One key of the viewer. Gemini 8/12: on Windows, Ctrl with the left
    /// Alt is the layout's AltGr where it has one (`Strg+Alt+Q` is `@` on a
    /// German layout), but winit filters only the right Alt (`VK_RMENU`,
    /// winit 0.30.13 `keyboard_layout.rs` `get_agnostic_mods`) and
    /// egui-winit drops text while Ctrl is held (`lib.rs` 794-800): such a
    /// key goes as the character the layout types, without the Ctrl and Alt
    /// the host holds, which are pressed again after it.
    pub(super) fn key_event(
        &mut self,
        key: Key,
        physical: Option<Key>,
        pressed: bool,
        repeat: bool,
        modifiers: Modifiers,
    ) {
        if pressed && modifiers.ctrl && modifiers.alt && self.caps.text {
            let text = physical
                .or(Some(key))
                .and_then(|place| platform::ctrl_alt_text(place, modifiers.shift));
            if let Some(text) = text {
                let plain = Modifiers {
                    ctrl: false,
                    alt: false,
                    ..modifiers
                };
                self.sync_modifiers(plain);
                self.text(&text);
                self.sync_modifiers(modifiers);
                if !repeat && !self.paste.typed.contains(&key) {
                    self.paste.typed.push(key);
                }
                return;
            }
        }
        if !pressed {
            if let Some(index) = self.paste.typed.iter().position(|typed| *typed == key) {
                self.paste.typed.remove(index);
                return;
            }
            self.paste_released(key, modifiers);
        }
        self.sync_modifiers(modifiers);
        self.key(key, pressed, repeat, modifiers);
    }

    /// The release of `key`: V with Ctrl, or Einfg with Umschalt and without
    /// Ctrl, whose press the host never got and whose press brought no
    /// `Paste`, sends the shortcut now; any release of V or Einfg ends the
    /// press (`handled`).
    pub(super) fn paste_released(&mut self, key: Key, mods: Modifiers) {
        let insert = key == Key::Insert;
        if key != Key::V && !insert {
            return;
        }
        let sent = key_usage_of(key).is_some_and(|usage| self.keys_down.contains(&usage));
        let shortcut = if insert {
            mods.shift && !mods.ctrl
        } else {
            mods.ctrl
        };
        if shortcut && !sent && !self.paste.handled {
            self.paste_shortcut(insert);
        }
        self.paste.handled = false;
        self.paste.press = None;
    }
}
