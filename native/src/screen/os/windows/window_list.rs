//! The windows of the Windows session for `LIST_WINDOWS` (SCR-M4, U16-6):
//! the windows that Alt+Tab shows. Source id `window:<HWND>` (decimal);
//! `apps` stays empty because the window list covers Windows completely.
//! Texts and counts are cut by `WindowList::bounded` in the host.
use std::ffi::c_void;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetShellWindow, GetWindow, GetWindowLongW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    GWL_EXSTYLE, GW_OWNER, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use super::monitors;
use crate::screen::core::window_list::{ListSource, WindowEntry, WindowList};

/// What decides whether Alt+Tab shows a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WindowFacts {
    pub(super) visible: bool,
    pub(super) has_title: bool,
    pub(super) cloaked: bool,
    pub(super) shell: bool,
    pub(super) has_owner: bool,
    pub(super) tool_window: bool,
    pub(super) app_window: bool,
}

/// The Alt+Tab rule: visible, with a title, not cloaked (a window of
/// another virtual desktop or a suspended app), not the shell window, either
/// without an owner or marked as an app window, and no tool window unless
/// marked as an app window.
pub(super) fn is_alt_tab_window(facts: &WindowFacts) -> bool {
    facts.visible
        && facts.has_title
        && !facts.cloaked
        && !facts.shell
        && (!facts.has_owner || facts.app_window)
        && (!facts.tool_window || facts.app_window)
}

/// The name of the program in `path`: the file name without extension.
pub(super) fn app_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.into(),
        _ => file.into(),
    }
}

/// The id of a window in `window:<id>`.
pub(super) fn window_id(hwnd: HWND) -> String {
    (hwnd.0 as usize).to_string()
}

unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the address of the `Vec` in `list`, alive for the
    // whole enumeration.
    let found = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    found.push(hwnd);
    BOOL(1)
}

fn facts_of(hwnd: HWND) -> WindowFacts {
    // SAFETY: plain queries on a window handle; a window that vanished
    // answers with the neutral value.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let mut cloaked = 0u32;
        let cloak = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        WindowFacts {
            visible: IsWindowVisible(hwnd).as_bool(),
            has_title: GetWindowTextLengthW(hwnd) > 0,
            cloaked: cloak.is_ok() && cloaked != 0,
            shell: hwnd == GetShellWindow(),
            has_owner: GetWindow(hwnd, GW_OWNER).is_ok(),
            tool_window: style & WS_EX_TOOLWINDOW.0 != 0,
            app_window: style & WS_EX_APPWINDOW.0 != 0,
        }
    }
}

fn title_of(hwnd: HWND) -> String {
    // SAFETY: the buffer holds the length the window reported plus the NUL.
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.clamp(0, length) as usize])
    }
}

/// Process id and program name of the window's process; the name is empty
/// when the process cannot be opened (a protected or elevated process).
fn process_of(hwnd: HWND) -> (u32, String) {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return (0, String::new());
    }
    // SAFETY: the process handle is closed below; the buffer length is
    // passed in units and updated by the call.
    let name = unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(process) => {
                let mut buffer = vec![0u16; 1024];
                let mut length = buffer.len() as u32;
                let queried = QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_WIN32,
                    PWSTR(buffer.as_mut_ptr()),
                    &mut length,
                );
                let _ = CloseHandle(process);
                match queried {
                    Ok(()) => app_name(&String::from_utf16_lossy(&buffer[..length as usize])),
                    Err(_) => String::new(),
                }
            }
            Err(_) => String::new(),
        }
    };
    (pid, name)
}

/// The windows Alt+Tab shows, in the order of the z-axis (topmost first),
/// with the limits of this host.
pub(super) fn list(limits: Vec<String>) -> WindowList {
    monitors::ensure_dpi_aware();
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `handles`, which outlives the call.
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HWND> as isize),
        );
    }
    // SAFETY: a plain query.
    let foreground = unsafe { GetForegroundWindow() };
    let windows = handles
        .into_iter()
        .filter(|hwnd| is_alt_tab_window(&facts_of(*hwnd)))
        .map(|hwnd| {
            let (pid, app) = process_of(hwnd);
            // SAFETY: plain queries on a window handle.
            let (monitor, minimized) = unsafe {
                (
                    monitors::id_of_handle(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)),
                    IsIconic(hwnd).as_bool(),
                )
            };
            WindowEntry {
                id: window_id(hwnd),
                title: title_of(hwnd),
                app,
                app_id: None,
                icon: None,
                pid: (pid != 0).then_some(pid),
                monitor,
                minimized,
                focused: hwnd == foreground,
            }
        })
        .collect();
    WindowList {
        windows,
        apps: Vec::new(),
        list_source: ListSource::EnumWindows,
        limits,
    }
}

#[cfg(test)]
#[path = "window_list_tests.rs"]
mod tests;
