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
    pub fn held(_key: super::Key) -> bool {
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
    /// The shortcut went out for this press: a `Paste` egui makes late
    /// sends nothing more (round 9 D9).
    pub(super) substituted: bool,
    /// Einfg, Entf, C, X or V let go in this frame (round 9 D8: one pressed
    /// and let go within a frame reads as up in `GetKeyState`).
    released: Vec<Key>,
    /// The modifiers at each `Copy`, `Cut` and `Paste` of this frame, in
    /// order (`clip_mods`).
    clip: std::collections::VecDeque<Modifiers>,
}

/// The modifiers an event of egui carries.
fn mods_of(event: &Event) -> Option<Modifiers> {
    match event {
        Event::Key { modifiers, .. }
        | Event::PointerButton { modifiers, .. }
        | Event::MouseWheel { modifiers, .. } => Some(*modifiers),
        _ => None,
    }
}

impl InputMapper {
    /// Whether `key` is down or was let go in this frame (Windows only:
    /// egui-winit makes `Copy`, `Cut` and `Paste` of Einfg and Entf there).
    fn struck(&self, key: Key) -> bool {
        cfg!(windows) && (platform::held(key) || self.paste.released.contains(&key))
    }

    /// Whether this frame's `Copy`, `Cut` or `Paste` came from `special`
    /// (Einfg, Entf) rather than `letter` (C, X, V): the letter decides when
    /// both are down (round 10: an Einfg held by chance made Strg+V Einfg).
    pub(super) fn variant(&self, special: Key, letter: Key) -> bool {
        self.struck(special) && !self.struck(letter)
    }

    /// The modifiers at the next `Copy`, `Cut` or `Paste` of this frame
    /// (round 10: those of the frame's end added one let go after it).
    pub(super) fn clip_mods(&mut self) -> Modifiers {
        self.paste.clip.pop_front().unwrap_or(self.paste.mods)
    }

    /// `Copy`: Strg+Einfg as itself where egui-winit made it `Copy` (Codex
    /// 8/11: a terminal copies on Strg+Einfg and interrupts on Strg+C),
    /// else Strg+C; each with the other modifiers held (round 10 B5).
    pub(super) fn copy(&mut self) {
        let held = self.clip_mods();
        if self.variant(Key::Insert, Key::C) {
            self.tap_with("insert", Modifiers { ctrl: true, ..held });
        } else {
            self.chord("c", held);
        }
    }

    /// Ctrl+V (`v`) or Umschalt+Einfg (`insert`) to the host, which pastes
    /// its own clipboard.
    fn paste_shortcut(&mut self, insert: bool) {
        self.paste.handled = true;
        self.paste.substituted = true;
        let held = self.paste.mods;
        if insert {
            self.tap_with(
                "insert",
                Modifiers {
                    shift: true,
                    ..held
                },
            );
        } else {
            self.chord("v", held);
        }
    }

    /// At the start of each frame: notes egui's `Paste` and, where the key
    /// state is known, sends a swallowed shortcut before its release.
    /// `frame_mods` are `NONE` while a toolbar field holds the keyboard.
    pub(super) fn missed_paste(&mut self, events: &[Event], mods: Modifiers) {
        // The modifiers at the end of the frame before (this frame's start).
        let start = std::mem::replace(&mut self.paste.mods, mods);
        if events.iter().any(|event| matches!(event, Event::Paste(_))) {
            self.paste.handled = true;
        }
        self.paste.released = events
            .iter()
            .filter_map(|event| match event {
                Event::Key {
                    key: key @ (Key::Insert | Key::Delete | Key::C | Key::X | Key::V),
                    pressed: false,
                    ..
                } => Some(*key),
                _ => None,
            })
            .collect();
        // The modifiers at each clipboard event: egui's `Copy`, `Cut` and
        // `Paste` carry none (egui 0.29.1 `input.rs` 382-388). A modifier
        // counts when it was down both at the last sample before (the event
        // before it in this frame, else the frame's start) and at the first
        // after (the event after it, else the frame's end): one let go or
        // pressed without an event between can only be missing, never one
        // that was not down at the press (unless let go and pressed again
        // within the frame around it).
        self.paste.clip.clear();
        for (index, event) in events.iter().enumerate() {
            if matches!(event, Event::Copy | Event::Cut | Event::Paste(_)) {
                let before = events[..index].iter().rev().find_map(mods_of);
                let after = events[index + 1..].iter().find_map(mods_of);
                let (before, after) = (before.unwrap_or(start), after.unwrap_or(mods));
                self.paste.clip.push_back(Modifiers {
                    ctrl: before.ctrl && after.ctrl,
                    shift: before.shift && after.shift,
                    alt: before.alt && after.alt,
                    ..after
                });
            }
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
        // Round 9 D9: also a press already let go waits the two frames and
        // `PASTE_WAIT` (X11 may report the keys before winit's events, so
        // egui's `Paste` came a frame after them); its release ends it.
        if press.frames < 2 || now < press.since + PASTE_WAIT {
            return;
        }
        self.paste.press = None;
        // Round 9 D1: a key the host got itself is no swallowed paste.
        let usage = key_usage(if insert { "insert" } else { "v" });
        let sent = usage.is_some_and(|usage| self.keys_down.contains(&usage));
        if !self.paste.handled && !sent {
            self.paste_shortcut(insert);
        }
    }

    /// A press waits for its decision (`missed_paste`): the view repaints.
    pub fn paste_pending(&self) -> bool {
        self.paste.press.is_some()
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
        self.paste.substituted = false;
        self.paste.press = None;
    }
}
