//! Chords, text and IME commits of the viewer's input mapping (moved out of
//! `screen_input_map.rs` to keep it below 500 lines).
use super::*;

impl InputMapper {
    /// Ctrl+C, X and V arrive as their own events without a key event.
    pub(super) fn chord(&mut self, name: &str) {
        let Some(hid_usage) = key_usage(name).filter(|_| self.caps.keyboard) else {
            return;
        };
        self.sync_modifiers(Modifiers {
            ctrl: true,
            ..self.mods
        });
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
        } else if self.mods.shift && !self.mods.ctrl {
            self.tap("insert");
        } else {
            self.chord("v");
        }
    }

    /// `Cut`: Strg+X, or (Windows) Umschalt+Entf as itself (Gemini 5/9:
    /// the chord made it Strg+Umschalt+X).
    pub(super) fn cut(&mut self) {
        if self.mods.shift && !self.mods.ctrl {
            self.tap("delete");
        } else {
            self.chord("x");
        }
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
