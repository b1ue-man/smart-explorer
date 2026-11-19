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
