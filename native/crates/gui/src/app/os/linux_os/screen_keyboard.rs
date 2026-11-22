//! Keyboard facts of the Linux viewer for the remote input (Gemini 5/9):
//! whether Ctrl+V is held (`screen_input_paste.rs`). On
//! X11 the server tells which keys are down (`QueryKeymap`); a Wayland
//! client learns keys only through its own focused surface, so there the
//! release decides alone.
//! winit picks Wayland when `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set
//! (winit 0.30 `platform_impl/linux/mod.rs` 736-744; se forces no backend).
use std::sync::{Mutex, PoisonError};

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// `XK_v` and `XK_V` (X11 `keysymdef.h`).
const KEYSYM_V: [u32; 2] = [0x0076, 0x0056];

/// The X11 connection and the keycodes that carry V.
struct Keys {
    connection: RustConnection,
    v: Vec<u8>,
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
    };
    for (index, syms) in mapping.keysyms.chunks(per).enumerate() {
        let code = u8::try_from(usize::from(first) + index).ok()?;
        if syms.iter().any(|sym| KEYSYM_V.contains(sym)) {
            keys.v.push(code);
        }
    }
    Some(keys)
}

/// egui-winit turns Strg+Einfg into `Copy` on Windows only: here it comes
/// as the key itself.
pub fn insert_held() -> bool {
    false
}

/// X11 leaves Ctrl and Alt to the layout: an AltGr character comes as text.
pub fn ctrl_alt_text(_physical: eframe::egui::Key, _shift: bool) -> Option<String> {
    None
}

/// How often the server is asked while Ctrl or Umschalt is held (Gemini
/// 8/12: once per frame was a round trip per frame); within `PASTE_WAIT`.
const QUERY_GAP: std::time::Duration = std::time::Duration::from_millis(25);

/// Ctrl+V is held now (X11 only). Umschalt+Einfg is no paste of
/// egui-winit here (`lib.rs` 1035-1039, Windows only): Einfg reaches the
/// host as its key, and a substitute pasted twice (round 9 D1).
pub fn paste_keys_held(ctrl: bool, _shift: bool) -> bool {
    if !ctrl || wayland() {
        return false;
    }
    // The last answer and when the server gave it.
    static LAST: Mutex<Option<(std::time::Instant, bool)>> = Mutex::new(None);
    let now = std::time::Instant::now();
    if let Some((at, held)) = *LAST.lock().unwrap_or_else(PoisonError::into_inner) {
        if now.duration_since(at) < QUERY_GAP {
            return held;
        }
    }
    let held = query();
    *LAST.lock().unwrap_or_else(PoisonError::into_inner) = Some((now, held));
    held
}

fn query() -> bool {
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
    down(&keys.v)
}
