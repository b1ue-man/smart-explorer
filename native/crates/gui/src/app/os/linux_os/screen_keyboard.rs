//! Keyboard facts of the Linux viewer for the remote input (Gemini 5/9).
//! AltGr needs nothing here: X11 and Wayland report it as level 3, so
//! egui-winit hands its character over as `Text`. Ctrl+V and Umschalt+Einfg
//! are swallowed by egui-winit for a clipboard without text, on X11 and
//! Wayland alike; on X11 the server tells which keys are down
//! (`QueryKeymap`). A Wayland client learns keys only through its own
//! focused surface, and egui-winit keeps the swallowed key to itself, so a
//! viewer window on Wayland cannot see it (it stays undetected there).
//! winit picks Wayland when `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set
//! (winit 0.30 `platform_impl/linux/mod.rs` 736-744; se forces no backend).
use std::sync::{Mutex, PoisonError};

use eframe::egui::Key;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// `XK_v`, `XK_V` and `XK_Insert` (X11 `keysymdef.h`).
const KEYSYM_V: [u32; 2] = [0x0076, 0x0056];
const KEYSYM_INSERT: u32 = 0xFF63;

pub fn altgr_held() -> bool {
    false
}

pub fn altgr_text(_physical: Key, _shift: bool) -> Option<String> {
    None
}

/// The X11 connection and the keycodes that carry V and Einfg.
struct Keys {
    connection: RustConnection,
    v: Vec<u8>,
    insert: Vec<u8>,
}

fn wayland() -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn open() -> Option<Keys> {
    let (connection, _) = x11rb::connect(None).ok()?;
    let setup = connection.setup();
    let (first, last) = (setup.min_keycode, setup.max_keycode);
    let count = last.checked_sub(first)?.checked_add(1)?;
    let mapping = connection
        .get_keyboard_mapping(first, count)
        .ok()?
        .reply()
        .ok()?;
    let per = usize::from(mapping.keysyms_per_keycode).max(1);
    let mut keys = Keys {
        connection,
        v: Vec::new(),
        insert: Vec::new(),
    };
    for (index, syms) in mapping.keysyms.chunks(per).enumerate() {
        let code = u8::try_from(usize::from(first) + index).ok()?;
        if syms.iter().any(|sym| KEYSYM_V.contains(sym)) {
            keys.v.push(code);
        }
        if syms.contains(&KEYSYM_INSERT) {
            keys.insert.push(code);
        }
    }
    Some(keys)
}

/// Ctrl+V, or Umschalt+Einfg without Ctrl, is held now (X11 only).
pub fn paste_keys_held(ctrl: bool, shift: bool) -> bool {
    if !(ctrl || shift) || wayland() {
        return false;
    }
    // One connection for the process (`None`: no X server, asked once).
    static KEYS: Mutex<Option<Option<Keys>>> = Mutex::new(None);
    let mut keys = KEYS.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(keys) = keys.get_or_insert_with(open).as_ref() else {
        return false;
    };
    let Some(map) = keys
        .connection
        .query_keymap()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return false;
    };
    let down = |codes: &[u8]| {
        codes
            .iter()
            .any(|code| map.keys[usize::from(*code / 8)] & (1 << (code % 8)) != 0)
    };
    (ctrl && down(&keys.v)) || (shift && !ctrl && down(&keys.insert))
}
