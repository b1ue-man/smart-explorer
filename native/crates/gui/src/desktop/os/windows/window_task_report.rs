//! What a failed wait of the DSK-M5 window checks shows (`window_task_env`):
//! the state of the child process with its exit status and the tail of its
//! standard error, and every top-level window the process owns, visible or
//! not. Without them a surface that ends before it has a window is
//! indistinguishable from one that never shows it.
use std::path::Path;
use std::process::Child;

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindowVisible,
    GWL_EXSTYLE, GWL_STYLE,
};

/// At most this many bytes of the end of the child's standard error.
const STDERR_TAIL: usize = 4000;

/// Whether the process still runs or how it ended, and its standard error.
pub(super) fn child_report(child: &mut Child, errors: &Path) -> String {
    let pid = child.id();
    let state = match child.try_wait() {
        Ok(Some(status)) => format!("beendet mit {status}"),
        Ok(None) => "läuft".to_string(),
        Err(error) => format!("Zustand unbekannt: {error}"),
    };
    let bytes = std::fs::read(errors).unwrap_or_default();
    let start = bytes.len().saturating_sub(STDERR_TAIL);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let text = text.trim();
    let text = if text.is_empty() { "(leer)" } else { text };
    format!("Prozess {pid} {state}; Stderr: {text}")
}

/// Class, visibility and styles of every top-level window of `pid`.
pub(super) fn windows_report(pid: u32) -> String {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        let (pid, rows) = &mut *(data as *mut (u32, Vec<String>));
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, &mut owner);
        if owner == *pid {
            let mut class = [0u16; 128];
            let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0);
            rows.push(format!(
                "{} sichtbar={} style={:#x} exstyle={:#x}",
                String::from_utf16_lossy(&class[..length as usize]),
                IsWindowVisible(hwnd) != 0,
                GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32,
            ));
        }
        1
    }
    let mut data: (u32, Vec<String>) = (pid, Vec::new());
    unsafe {
        EnumWindows(
            Some(collect),
            &mut data as *mut (u32, Vec<String>) as LPARAM,
        )
    };
    if data.1.is_empty() {
        "kein Fenster des Prozesses".to_string()
    } else {
        format!("Fenster des Prozesses: [{}]", data.1.join("; "))
    }
}
