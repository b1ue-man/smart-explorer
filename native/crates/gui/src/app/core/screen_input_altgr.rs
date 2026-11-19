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
use super::*;

#[cfg(windows)]
#[path = "../os/windows/screen_keyboard.rs"]
mod platform;
#[cfg(not(windows))]
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

    /// Ctrl+V or Umschalt+Einfg that egui-winit swallowed (a clipboard
    /// without text): an empty `Paste` joins the frame once per press, and
    /// `paste` sends the shortcut, so the host pastes its own clipboard. A
    /// frame with egui's own `Paste` (the clipboard held text) adds none and
    /// marks the press as handled, so nothing goes out twice; `frame_mods`
    /// are `NONE` while a toolbar field holds the keyboard.
    pub(super) fn missed_paste(&mut self, events: &[Event], mods: Modifiers) -> Option<Vec<Event>> {
        let held = platform::paste_keys_held(mods.ctrl, mods.shift);
        let came = events.iter().any(|event| matches!(event, Event::Paste(_)));
        let missed = held && !self.paste_held && !came;
        self.paste_held = held;
        let empty = Event::Paste(String::new());
        missed.then(|| events.iter().cloned().chain([empty]).collect())
    }
}
