//! Keyboard facts of the Windows viewer for the remote input (Gemini 5/9):
//! whether Ctrl+V or Umschalt+Einfg is held (`screen_input_paste.rs`).
//! `GetKeyState` answers for the calling thread as of the last keyboard
//! message it took from its queue; the window's thread runs egui, which
//! read those messages before the frame (eframe's event loop).
use eframe::egui::Key;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, MapVirtualKeyExW, ToUnicodeEx, MAPVK_VSC_TO_VK_EX, VK_CAPITAL,
    VK_CONTROL, VK_DELETE, VK_INSERT, VK_LCONTROL, VK_LMENU, VK_MENU, VK_RMENU, VK_SHIFT, VK_V,
};

/// `ToUnicodeEx` flag: leave the kernel's keyboard state (a pending dead
/// key) as it was (Windows 10 1607 and later).
const NO_STATE_CHANGE: u32 = 1 << 2;

fn down(key: u16) -> bool {
    // SAFETY: no pointers; the calling thread's key state.
    let state = unsafe { GetKeyState(i32::from(key)) };
    state < 0
}

/// Einfg is held now: egui-winit makes Strg+Einfg the same `Copy` as
/// Strg+C (`lib.rs` 1029), and the host gets the keys that were pressed.
pub fn insert_held() -> bool {
    down(VK_INSERT)
}

/// Entf is held now: egui-winit makes Umschalt+Entf (also with Strg) a
/// `Cut` like Strg+X (`lib.rs` 1025-1027, round 10 B5).
pub fn delete_held() -> bool {
    down(VK_DELETE)
}

/// Ctrl+V, or Umschalt+Einfg without Ctrl, is held now.
pub fn paste_keys_held(ctrl: bool, shift: bool) -> bool {
    (ctrl && down(VK_V)) || (shift && !ctrl && down(VK_INSERT))
}

/// Gemini 8/12: the character Ctrl with the left Alt types at `physical`'s
/// place (and Umschalt when `shift`) under the window's layout, the way
/// Windows lets Ctrl+Alt stand for AltGr where a layout has one; `None`
/// with the right Alt (winit already makes that text), on a layout
/// without AltGr, for a dead key or a control character.
pub fn ctrl_alt_text(physical: Key, shift: bool) -> Option<String> {
    if !down(VK_LMENU) || down(VK_RMENU) {
        return None;
    }
    let scan = scan_code(physical)?;
    // SAFETY: 0 names the calling thread, which owns the viewer's window.
    let layout = unsafe { GetKeyboardLayout(0) };
    // SAFETY: plain values; 0 means the layout has no key at that place.
    let virtual_key = unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, layout) };
    if virtual_key == 0 {
        return None;
    }
    let mut state = [0u8; 256];
    for held in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_LMENU] {
        state[usize::from(held)] = 0x80;
    }
    if shift {
        state[usize::from(VK_SHIFT)] = 0x80;
    }
    // SAFETY: no pointers; bit 0 is the toggle state of Caps Lock.
    if unsafe { GetKeyState(i32::from(VK_CAPITAL)) } & 1 != 0 {
        state[usize::from(VK_CAPITAL)] = 1;
    }
    let mut buffer = [0u16; 8];
    // SAFETY: `state` holds the 256 entries the call reads, `buffer` the
    // 8 units it may write (its length is passed); `NO_STATE_CHANGE` keeps
    // the kernel's dead-key state.
    let written = unsafe {
        ToUnicodeEx(
            virtual_key,
            scan,
            state.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            NO_STATE_CHANGE,
            layout,
        )
    };
    // Negative: a dead key; 0: no character (a layout without AltGr).
    let written = usize::try_from(written).ok().filter(|count| *count > 0)?;
    let text = String::from_utf16(&buffer[..written.min(buffer.len())]).ok()?;
    (!text.chars().any(char::is_control)).then_some(text)
}

/// The set-1 scan code of the key `key` names: egui names a physical key
/// by its place on a US keyboard (egui-winit `key_from_key_code`).
fn scan_code(key: Key) -> Option<u32> {
    const KEYS: [(Key, u32); 47] = [
        (Key::Q, 0x10),
        (Key::W, 0x11),
        (Key::E, 0x12),
        (Key::R, 0x13),
        (Key::T, 0x14),
        (Key::Y, 0x15),
        (Key::U, 0x16),
        (Key::I, 0x17),
        (Key::O, 0x18),
        (Key::P, 0x19),
        (Key::A, 0x1E),
        (Key::S, 0x1F),
        (Key::D, 0x20),
        (Key::F, 0x21),
        (Key::G, 0x22),
        (Key::H, 0x23),
        (Key::J, 0x24),
        (Key::K, 0x25),
        (Key::L, 0x26),
        (Key::Z, 0x2C),
        (Key::X, 0x2D),
        (Key::C, 0x2E),
        (Key::V, 0x2F),
        (Key::B, 0x30),
        (Key::N, 0x31),
        (Key::M, 0x32),
        (Key::Num1, 0x02),
        (Key::Num2, 0x03),
        (Key::Num3, 0x04),
        (Key::Num4, 0x05),
        (Key::Num5, 0x06),
        (Key::Num6, 0x07),
        (Key::Num7, 0x08),
        (Key::Num8, 0x09),
        (Key::Num9, 0x0A),
        (Key::Num0, 0x0B),
        (Key::Minus, 0x0C),
        (Key::Equals, 0x0D),
        (Key::OpenBracket, 0x1A),
        (Key::CloseBracket, 0x1B),
        (Key::Semicolon, 0x27),
        (Key::Quote, 0x28),
        (Key::Backtick, 0x29),
        (Key::Backslash, 0x2B),
        (Key::Comma, 0x33),
        (Key::Period, 0x34),
        (Key::Slash, 0x35),
    ];
    KEYS.iter()
        .find(|(named, _)| *named == key)
        .map(|(_, code)| *code)
}
