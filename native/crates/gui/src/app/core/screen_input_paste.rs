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
    pub fn paste_keys_held(_ctrl: bool, _shift: bool) -> bool {
        false
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
}

impl InputMapper {
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
