//! Keyboard facts of the Windows viewer for the remote input (Gemini 5/9):
//! whether Ctrl+V or Umschalt+Einfg is held (`screen_input_paste.rs`).
//! `GetKeyState` answers for the calling thread as of the last keyboard
//! message it took from its queue; the window's thread runs egui, which
//! read those messages before the frame (eframe's event loop).
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_INSERT, VK_V};

fn down(key: u16) -> bool {
    // SAFETY: no pointers; the calling thread's key state.
    let state = unsafe { GetKeyState(i32::from(key)) };
    state < 0
}

/// Ctrl+V, or Umschalt+Einfg without Ctrl, is held now.
pub fn paste_keys_held(ctrl: bool, shift: bool) -> bool {
    (ctrl && down(VK_V)) || (shift && !ctrl && down(VK_INSERT))
}
