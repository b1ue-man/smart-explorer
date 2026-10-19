//! Monitors of the Windows session for the screen host (SCR-M4, U16-6 and
//! U16-8): the id `DISPLAYn` of a GDI output (the source is `monitor:DISPLAYn`),
//! the name `Bildschirm n`, position and size in physical pixels of the
//! virtual screen, sorted by `y`, then `x`. The process counts its pixels as
//! physical ones (`ensure_dpi_aware`), so a monitor at 150 % reports its
//! real resolution.
use std::sync::OnceLock;

use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use crate::screen::core::window_list::MonitorInfo;

/// The prefix of a GDI device name (`\\.\DISPLAY1`).
const DEVICE_PREFIX: &str = r"\\.\";

/// One monitor with the handle and the device name Windows knows it by.
pub(super) struct MonitorEntry {
    pub(super) info: MonitorInfo,
    pub(super) handle: HMONITOR,
    /// `\\.\DISPLAYn`, as DXGI names the output.
    pub(super) device: String,
}

/// Makes this process count in physical pixels (per-monitor aware v2),
/// once. A process that is already aware (the desktop app's manifest) keeps
/// its setting: the refusal is not an error.
pub(super) fn ensure_dpi_aware() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        // SAFETY: the constant is one of the documented contexts.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    });
}

/// The id of a GDI output: its device name without `\\.\`.
pub(super) fn id_of_device(device: &str) -> String {
    device.strip_prefix(DEVICE_PREFIX).unwrap_or(device).into()
}

/// `Bildschirm n` after the number at the end of the id (`DISPLAY2`).
pub(super) fn name_of_id(id: &str) -> String {
    let digits: String = id
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        "Bildschirm".into()
    } else {
        format!("Bildschirm {digits}")
    }
}

fn text(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

unsafe extern "system" fn collect(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the address of the `Vec` `handles` below, alive for
    // the whole enumeration.
    let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    BOOL(1)
}

fn entry_of(handle: HMONITOR) -> Option<MonitorEntry> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `MONITORINFOEXW` starts with a `MONITORINFO` and `cbSize`
    // names the larger structure.
    let read =
        unsafe { GetMonitorInfoW(handle, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
    if !read.as_bool() {
        return None;
    }
    let device = text(&info.szDevice);
    let id = id_of_device(&device);
    let rect = info.monitorInfo.rcMonitor;
    Some(MonitorEntry {
        info: MonitorInfo {
            name: name_of_id(&id),
            id,
            x: rect.left,
            y: rect.top,
            width: u32::try_from(rect.right - rect.left).unwrap_or(0),
            height: u32::try_from(rect.bottom - rect.top).unwrap_or(0),
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        },
        handle,
        device,
    })
}

/// Every monitor, sorted by `y`, then `x`; none without a display.
pub(super) fn enumerate() -> Vec<MonitorEntry> {
    ensure_dpi_aware();
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into `handles`, which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC(std::ptr::null_mut()),
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }
    let mut monitors: Vec<MonitorEntry> = handles.into_iter().filter_map(entry_of).collect();
    monitors.sort_by_key(|entry| (entry.info.y, entry.info.x));
    monitors
}

/// The monitors of `HELLO_OK`.
pub(super) fn infos() -> Vec<MonitorInfo> {
    enumerate().into_iter().map(|entry| entry.info).collect()
}

/// The monitor with `id`, read now (a handle can change with the display
/// setup).
pub(super) fn find(id: &str) -> Option<MonitorEntry> {
    enumerate().into_iter().find(|entry| entry.info.id == id)
}

/// The id of the monitor behind `handle` (the monitor of a window).
pub(super) fn id_of_handle(handle: HMONITOR) -> Option<String> {
    entry_of(handle).map(|entry| entry.info.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_monitor_names_follow_the_device_number() {
        assert_eq!(id_of_device(r"\\.\DISPLAY12"), "DISPLAY12");
        assert_eq!(id_of_device("DISPLAY3"), "DISPLAY3");
        assert_eq!(name_of_id("DISPLAY12"), "Bildschirm 12");
        assert_eq!(name_of_id("WinDisc"), "Bildschirm");
    }
}
