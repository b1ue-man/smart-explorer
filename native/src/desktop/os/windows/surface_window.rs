//! The surface window of one monitor (DSK-M5, C1.2, L3): a borderless tool
//! window over the Explorer desktop and below every program window, without
//! task-bar and Alt+Tab entry. winit creates it without decorations and
//! minimize button (`with_minimize_button(false)`: otherwise winit sets
//! `WS_MINIMIZEBOX` and Win+D would minimize it, R8, R10). After creation
//! the window gets `WS_EX_TOOLWINDOW` without `WS_EX_APPWINDOW`, the exact
//! physical `rcMonitor` and the bottom of the z-order, and a subclass
//! (`SetWindowSubclass`) with the three rules of C1.2:
//! 1. `WM_WINDOWPOSCHANGING` sets `hwndInsertAfter = HWND_BOTTOM` and keeps
//!    the flags, so an activation (a click) leaves the window at the bottom
//!    (Annahme A5); a move or resize keeps the monitor's rectangle, also
//!    when winit answers `WM_DPICHANGED` on a monitor of another scale;
//! 2. `WM_STYLECHANGING` keeps `WS_EX_TOOLWINDOW` and drops
//!    `WS_EX_APPWINDOW` in every new extended style (winit sets it again,
//!    R7; Annahme A3), and drops `WS_MINIMIZEBOX` in every new style;
//! 3. `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE` (`SPI_SETWORKAREA`,
//!    `SPI_SETDESKWALLPAPER`) and the registered `TaskbarCreated` (Explorer
//!    restarted; the window goes to the bottom again) tell the view.
//!
//! The subclass also notices Ctrl+V and Shift+Insert while the clipboard
//! holds no text (a file copy in Explorer): egui 0.29 turns a paste key
//! into its paste event only for text, so the view hears it here.
use std::cell::Cell;
use std::sync::Arc;

use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_INSERT, VK_MENU, VK_SHIFT, VK_V,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, RegisterWindowMessageW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    GWL_STYLE, HWND_BOTTOM, SPI_SETDESKWALLPAPER, SPI_SETWORKAREA, STYLESTRUCT, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, WINDOWPOS, WM_DISPLAYCHANGE,
    WM_KEYDOWN, WM_NCDESTROY, WM_SETTINGCHANGE, WM_STYLECHANGING, WM_WINDOWPOSCHANGING,
    WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MINIMIZEBOX,
};

use super::monitors;
use crate::app::desktop_surface_host::{NativeWindow, SurfaceSignal, SurfaceSignals};

/// Identifies this subclass on the window ("SEDS").
const SUBCLASS_ID: usize = 0x5345_4453;

/// Lives on the window's thread from `attach_surface` to `WM_NCDESTROY`.
struct SubclassState {
    signals: Arc<SurfaceSignals>,
    /// The registered message `TaskbarCreated`; 0 when it could not be had.
    taskbar_created: u32,
    /// Key of the surface's monitor.
    monitor: String,
    /// `rcMonitor` of that monitor, physical pixels, read again on a change.
    rect: Cell<Option<[i32; 4]>>,
}

impl SubclassState {
    /// Reads the monitor again and puts the window back on it, at the bottom.
    fn refit(&self, hwnd: HWND) {
        self.rect
            .set(monitors::info_for(&self.monitor).map(|info| info.rect));
        to_bottom(hwnd, self.rect.get());
    }
}

/// Rule 2 for `GWL_EXSTYLE`.
pub(crate) fn enforced_ex_style(style: u32) -> u32 {
    (style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW
}

/// Rule 2 for `GWL_STYLE`: never a minimize box.
pub(crate) fn enforced_style(style: u32) -> u32 {
    style & !WS_MINIMIZEBOX
}

fn pressed(key: u16) -> bool {
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

/// Ctrl+V or Shift+Insert as egui reads a paste key.
fn paste_key(key: WPARAM) -> bool {
    let key = key as u16;
    let ctrl_v = key == VK_V && pressed(VK_CONTROL) && !pressed(VK_MENU);
    let shift_insert = key == VK_INSERT && pressed(VK_SHIFT) && !pressed(VK_CONTROL);
    ctrl_v || shift_insert
}

fn text_on_clipboard() -> bool {
    // Windows offers `CF_UNICODETEXT` also for `CF_TEXT` and `CF_OEMTEXT`.
    unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) }.is_ok()
}

fn to_bottom(hwnd: HWND, rect: Option<[i32; 4]>) {
    let (x, y, width, height, place) = match rect {
        Some([left, top, right, bottom]) => (left, top, right - left, bottom - top, 0),
        None => (0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE),
    };
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            x,
            y,
            width,
            height,
            place | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        drop(Box::from_raw(data as *mut SubclassState));
        return result;
    }
    let state = &*(data as *const SubclassState);
    match message {
        WM_WINDOWPOSCHANGING if lparam != 0 => {
            let position = &mut *(lparam as *mut WINDOWPOS);
            position.hwndInsertAfter = HWND_BOTTOM;
            if let Some([left, top, right, bottom]) = state.rect.get() {
                if position.flags & SWP_NOMOVE == 0 {
                    (position.x, position.y) = (left, top);
                }
                if position.flags & SWP_NOSIZE == 0 {
                    (position.cx, position.cy) = (right - left, bottom - top);
                }
            }
        }
        WM_STYLECHANGING if lparam != 0 => {
            let change = &mut *(lparam as *mut STYLESTRUCT);
            match wparam as i32 {
                GWL_EXSTYLE => change.styleNew = enforced_ex_style(change.styleNew),
                GWL_STYLE => change.styleNew = enforced_style(change.styleNew),
                _ => {}
            }
        }
        WM_DISPLAYCHANGE => {
            state.refit(hwnd);
            state.signals.raise(SurfaceSignal::Display);
        }
        WM_SETTINGCHANGE => match wparam as u32 {
            SPI_SETWORKAREA => state.signals.raise(SurfaceSignal::Display),
            SPI_SETDESKWALLPAPER => state.signals.raise(SurfaceSignal::Wallpaper),
            _ => {}
        },
        WM_KEYDOWN if paste_key(wparam) && !text_on_clipboard() => {
            state.signals.raise(SurfaceSignal::Paste);
        }
        _ if state.taskbar_created != 0 && message == state.taskbar_created => {
            state.refit(hwnd);
            state.signals.raise(SurfaceSignal::Display);
            state.signals.raise(SurfaceSignal::Wallpaper);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

/// Makes the window of a surface the lowest tool window on the monitor
/// with `monitor` and reports its events to `signals`.
pub(crate) fn attach_surface(
    window: NativeWindow,
    monitor: &str,
    signals: Arc<SurfaceSignals>,
) -> Result<(), String> {
    let NativeWindow::Win32 { hwnd } = window else {
        return Err(format!("kein Win32-Fenster: {window:?}"));
    };
    let hwnd = hwnd as HWND;
    let name: Vec<u16> = "TaskbarCreated".encode_utf16().chain([0]).collect();
    let taskbar_created = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    let state = Box::into_raw(Box::new(SubclassState {
        signals,
        taskbar_created,
        monitor: monitor.to_string(),
        rect: Cell::new(None),
    }));
    if unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, state as usize) } == 0 {
        // SAFETY: the subclass was not installed, so nothing else owns it.
        drop(unsafe { Box::from_raw(state) });
        return Err(format!(
            "SetWindowSubclass: {}",
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, enforced_ex_style(extended) as isize);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if enforced_style(style) != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, enforced_style(style) as isize);
        }
        // The new styles take effect with the frame change.
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
    // The whole monitor, so no Explorer desktop shows under a translucent
    // or hidden task bar; the grid keeps to `rcWork`.
    // SAFETY: installed above; the window procedure runs on this thread.
    unsafe { (*state).refit(hwnd) };
    Ok(())
}
