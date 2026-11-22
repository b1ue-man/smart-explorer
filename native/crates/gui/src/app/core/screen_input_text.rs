//! Chords, text and IME commits of the viewer's input mapping (moved out of
//! `screen_input_map.rs` to keep it below 500 lines).
use super::*;

impl InputMapper {
    /// Ctrl+C, X and V arrive as their own events without a key event;
    /// Umschalt and Alt go along as held now (round 10 B8: Strg+Umschalt+V
    /// pressed within one frame came as Strg+V).
    pub(super) fn chord(&mut self, name: &str) {
        let Some(hid_usage) = key_usage(name).filter(|_| self.caps.keyboard) else {
            return;
        };
        let held = self.held_now();
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
        if !self.caps.keyboard {
            if self.caps.text {
                self.text(text);
            }
        } else if self.paste.substituted {
            // Round 9 D9: the shortcut went out already for this press.
        } else if self.struck(Key::Insert) {
            let held = self.held_now();
            self.tap_with(
                "insert",
                Modifiers {
                    shift: true,
                    ..held
                },
            );
        } else {
            self.chord("v");
        }
    }

    /// The modifiers held around a `Copy`, `Cut` or `Paste`: those sent
    /// before this frame and the frame's own (Gemini 8/12: these events come
    /// before the frame's modifiers are sent).
    pub(super) fn held_now(&self) -> Modifiers {
        let frame = self.paste.mods;
        Modifiers {
            ctrl: self.mods.ctrl || frame.ctrl,
            shift: self.mods.shift || frame.shift,
            alt: self.mods.alt || frame.alt,
            ..self.mods
        }
    }

    /// `Cut`: (Windows) Umschalt+Entf as itself, with Strg when held (round
    /// 10 B5: egui-winit makes Strg+Umschalt+Entf a `Cut` too), else Strg+X.
    pub(super) fn cut(&mut self) {
        if self.struck(Key::Delete) {
            let held = self.held_now();
            self.tap_with(
                "delete",
                Modifiers {
                    shift: true,
                    ..held
                },
            );
        } else {
            self.chord("x");
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
