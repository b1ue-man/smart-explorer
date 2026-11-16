//! Refresh rate of the monitor the viewer's window is on (Windows): the
//! process's foreground window, else its first visible top-level window,
//! else the primary monitor; `EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)`
//! of that monitor's device. `dmDisplayFrequency` 0 and 1 mean „hardware
//! default“ (Microsoft, DEVMODEW) and count as unknown.
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS,
    MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTOPRIMARY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
};

fn owned(window: HWND) -> bool {
    let mut process = 0u32;
    // SAFETY: any window handle; the id is written into `process`.
    unsafe { GetWindowThreadProcessId(window, &mut process) };
    process == std::process::id()
}

unsafe extern "system" fn first_visible(window: HWND, found: LPARAM) -> BOOL {
    // SAFETY: `found` points at the `HWND` of `own_window` for the whole enumeration.
    let found = unsafe { &mut *(found as *mut HWND) };
    // SAFETY: a window handle the enumeration supplies.
    if owned(window) && unsafe { IsWindowVisible(window) } != 0 {
        *found = window;
        return 0;
    }
    1
}

fn own_window() -> HWND {
    // SAFETY: no arguments.
    let foreground = unsafe { GetForegroundWindow() };
    if !foreground.is_null() && owned(foreground) {
        return foreground;
    }
    let mut found: HWND = std::ptr::null_mut();
    // SAFETY: the callback only writes `found`, which outlives the call.
    unsafe { EnumWindows(Some(first_visible), &mut found as *mut HWND as LPARAM) };
    found
}

pub fn refresh_millihertz() -> Option<u32> {
    // SAFETY: a null window yields the primary monitor with this flag.
    let monitor = unsafe { MonitorFromWindow(own_window(), MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_null() {
        return None;
    }
    // SAFETY: plain C structures; zero is a valid start for both.
    let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` is a MONITORINFOEXW whose size `cbSize` names.
    if unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    } == 0
    {
        return None;
    }
    // SAFETY: as above.
    let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
    mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    // SAFETY: `szDevice` is the NUL-terminated device name GetMonitorInfoW wrote.
    let read =
        unsafe { EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) };
    (read != 0 && mode.dmDisplayFrequency > 1).then(|| mode.dmDisplayFrequency * 1000)
}
