//! Chords, text and IME commits of the viewer's input mapping (moved out of
//! `screen_input_map.rs` to keep it below 500 lines).
use super::*;

impl InputMapper {
    /// Ctrl+C, X and V arrive as their own events without a key event;
    /// Umschalt and Alt go along as `held` at that event (round 10 B8,
    /// `clip_mods`).
    pub(super) fn chord(&mut self, name: &str, held: Modifiers) {
        let Some(hid_usage) = key_usage(name).filter(|_| self.caps.keyboard) else {
            return;
        };
        self.sync_modifiers(Modifiers { ctrl: true, ..held });
        self.emit(InputEvent::Key {
            hid_usage,
            down: true,
        });
        self.emit(InputEvent::Key {
            hid_usage,
            down: false,
        });
    }

    /// egui-winit turns Ctrl+V and (Windows) Umschalt+Einfg into `Paste`
    /// with the viewer's clipboard (Gemini 5/9): a host with keys gets the
    /// shortcut the user pressed (its own clipboard), Umschalt+Einfg as
    /// itself instead of Strg+Umschalt+V; a host that takes text only
    /// (Android) gets the text, where the chord did nothing.
    pub(super) fn paste(&mut self, text: &str) {
        let held = self.clip_mods();
        if !self.caps.keyboard {
            if self.caps.text {
                self.text(text);
            }
        } else if self.paste.substituted {
            // Round 9 D9: the shortcut went out already for this press.
        } else if self.variant(Key::Insert, Key::V) {
            let shifted = Modifiers {
                shift: true,
                ..held
            };
            self.tap_with("insert", shifted);
        } else {
            self.chord("v", held);
        }
    }

    /// `Cut`: (Windows) Umschalt+Entf as itself, with Strg when held (round
    /// 10 B5: egui-winit makes Strg+Umschalt+Entf a `Cut` too), else Strg+X.
    pub(super) fn cut(&mut self) {
        let held = self.clip_mods();
        if self.variant(Key::Delete, Key::X) {
            let shifted = Modifiers {
                shift: true,
                ..held
            };
            self.tap_with("delete", shifted);
        } else {
            self.chord("x", held);
        }
    }

    /// `name` under `mods` on the host (round 9 D2: Entf went without the
    /// Umschalt of the frame and deleted instead of cutting).
    pub(super) fn tap_with(&mut self, name: &str, mods: Modifiers) {
        self.sync_modifiers(mods);
        self.tap(name);
    }

    /// One press and release of the key `name` under the held modifiers.
    pub(super) fn tap(&mut self, name: &str) {
        let Some(hid_usage) = key_usage(name).filter(|_| self.caps.keyboard) else {
            return;
        };
        for down in [true, false] {
            self.emit(InputEvent::Key { hid_usage, down });
        }
    }

    /// `Text` events of at most `MAX_TEXT_BYTES`, without NUL.
    pub(super) fn text(&mut self, text: &str) {
        let mut rest: String = text.chars().filter(|c| *c != '\0').collect();
        while !rest.is_empty() {
            let mut end = rest.len().min(MAX_TEXT_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let tail = rest.split_off(end);
            let head = std::mem::replace(&mut rest, tail);
            self.emit(InputEvent::Text { text: head });
        }
    }

    pub(super) fn ime(&mut self, ime: &ImeEvent) {
        match ime {
            ImeEvent::Commit(text) => {
                self.preedit = None;
                if self.caps.text {
                    self.text(text);
                }
            }
            ImeEvent::Preedit(text) => {
                self.preedit = (!text.is_empty()).then(|| text.clone());
            }
            ImeEvent::Enabled => {}
            ImeEvent::Disabled => self.preedit = None,
        }
    }
}
