//! xkbcommon at run time (DSK-M7, U37-11): `xkbcommon-dl` loads
//! `libxkbcommon.so.0` when the first keyboard appears, so the program links
//! no keyboard library at build time (R78). Without the library the process
//! runs without a keyboard (`LayerKeyboard::Off`), it does not crash.
use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::raw::c_char;
use std::ptr::NonNull;

use eframe::egui::Modifiers;
use xkbcommon_dl::{
    xkb_context, xkb_context_flags, xkb_keymap, xkb_keymap_compile_flags, xkb_keymap_format,
    xkb_state, xkb_state_component, xkbcommon_option, XkbCommon, XKB_MOD_NAME_ALT,
    XKB_MOD_NAME_CTRL, XKB_MOD_NAME_SHIFT,
};

use super::keysyms::modifiers_of;

/// Offset of the evdev key codes to xkb key codes.
const EVDEV_OFFSET: u32 = 8;

/// The keymap the compositor sent and the state of the modifiers.
pub(super) struct Xkb {
    lib: &'static XkbCommon,
    context: NonNull<xkb_context>,
    keymap: Option<NonNull<xkb_keymap>>,
    state: Option<NonNull<xkb_state>>,
}

impl Xkb {
    /// `None` when `libxkbcommon.so.0` or its context is not available.
    pub(super) fn new() -> Option<Xkb> {
        let lib = xkbcommon_option()?;
        // SAFETY: plain constructor call of the loaded library.
        let context = NonNull::new(unsafe {
            (lib.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS)
        })?;
        Some(Xkb {
            lib,
            context,
            keymap: None,
            state: None,
        })
    }

    /// `wl_keyboard.keymap` (format `xkb_v1`): reads `size` bytes of `fd`
    /// (the string with its terminating NUL) and builds keymap and state; a
    /// keymap that does not compile leaves the keyboard without keys.
    pub(super) fn set_keymap(&mut self, fd: OwnedFd, size: usize) -> bool {
        self.clear();
        let mut text = Vec::with_capacity(size.saturating_add(1));
        if File::from(fd)
            .take(size as u64)
            .read_to_end(&mut text)
            .is_err()
        {
            return false;
        }
        if text.last() != Some(&0) {
            text.push(0);
        }
        // SAFETY: `text` is NUL-terminated and outlives the call.
        let keymap = unsafe {
            (self.lib.xkb_keymap_new_from_string)(
                self.context.as_ptr(),
                text.as_ptr().cast::<c_char>(),
                xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        let Some(keymap) = NonNull::new(keymap) else {
            return false;
        };
        // SAFETY: `keymap` is valid; the state keeps its own reference.
        let state = NonNull::new(unsafe { (self.lib.xkb_state_new)(keymap.as_ptr()) });
        self.keymap = Some(keymap);
        self.state = state;
        state.is_some()
    }

    fn clear(&mut self) {
        // SAFETY: both pointers came from this library and are released once.
        unsafe {
            if let Some(state) = self.state.take() {
                (self.lib.xkb_state_unref)(state.as_ptr());
            }
            if let Some(keymap) = self.keymap.take() {
                (self.lib.xkb_keymap_unref)(keymap.as_ptr());
            }
        }
    }

    /// `wl_keyboard.modifiers`.
    pub(super) fn update_mask(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(state) = self.state {
            // SAFETY: `state` is valid.
            unsafe {
                (self.lib.xkb_state_update_mask)(
                    state.as_ptr(),
                    depressed,
                    latched,
                    locked,
                    0,
                    0,
                    group,
                );
            }
        }
    }

    /// The keysym of a key (evdev code) in the current layout and state.
    pub(super) fn sym(&self, key: u32) -> u32 {
        match self.state {
            // SAFETY: `state` is valid.
            Some(state) => unsafe {
                (self.lib.xkb_state_key_get_one_sym)(state.as_ptr(), key + EVDEV_OFFSET)
            },
            None => 0,
        }
    }

    /// The text a key produces now (dead keys and compose sequences come
    /// out uncomposed: no input method, see `desktop-launcher.md`).
    pub(super) fn text(&self, key: u32) -> Option<String> {
        let state = self.state?;
        let code = key + EVDEV_OFFSET;
        // SAFETY: the first call with a null buffer returns the length.
        let length = unsafe {
            (self.lib.xkb_state_key_get_utf8)(state.as_ptr(), code, std::ptr::null_mut(), 0)
        };
        let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
        let mut buffer = vec![0u8; length + 1];
        // SAFETY: the buffer holds `length + 1` bytes.
        let written = unsafe {
            (self.lib.xkb_state_key_get_utf8)(
                state.as_ptr(),
                code,
                buffer.as_mut_ptr().cast::<c_char>(),
                buffer.len(),
            )
        };
        if usize::try_from(written).ok() != Some(length) {
            return None;
        }
        buffer.truncate(length);
        String::from_utf8(buffer).ok()
    }

    /// Whether the keymap lets the key repeat.
    pub(super) fn repeats(&self, key: u32) -> bool {
        match self.keymap {
            // SAFETY: `keymap` is valid.
            Some(keymap) => unsafe {
                (self.lib.xkb_keymap_key_repeats)(keymap.as_ptr(), key + EVDEV_OFFSET) != 0
            },
            None => false,
        }
    }

    fn mod_active(&self, name: &[u8]) -> bool {
        match self.state {
            // SAFETY: `state` is valid and `name` is NUL-terminated.
            Some(state) => unsafe {
                (self.lib.xkb_state_mod_name_is_active)(
                    state.as_ptr(),
                    name.as_ptr().cast::<c_char>(),
                    xkb_state_component::XKB_STATE_MODS_EFFECTIVE,
                ) > 0
            },
            None => false,
        }
    }

    /// Alt (`Mod1`), Control and Shift of the current state.
    pub(super) fn modifiers(&self) -> Modifiers {
        modifiers_of(
            self.mod_active(XKB_MOD_NAME_ALT),
            self.mod_active(XKB_MOD_NAME_CTRL),
            self.mod_active(XKB_MOD_NAME_SHIFT),
        )
    }
}

impl Drop for Xkb {
    fn drop(&mut self) {
        self.clear();
        // SAFETY: the context came from this library and is released once.
        unsafe { (self.lib.xkb_context_unref)(self.context.as_ptr()) };
    }
}
