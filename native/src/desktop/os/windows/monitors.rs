//! Monitors of the Windows session (DSK-M5, C1.2): every display monitor
//! with its key, rectangle, work area and name.
//!
//! The key is the first 16 hex characters of SHA-256 over the monitor's
//! device interface path (`\\?\DISPLAY#…#{e6f07b5f-…}`), the ID that
//! `IDesktopWallpaper::GetMonitorDevicePathAt` names. The path comes from
//! `EnumDisplayDevicesW` with `EDD_GET_DEVICE_INTERFACE_NAME`, so the key
//! never depends on coordinates: the windowless shell is not DPI aware, the
//! surfaces are (winit), and both derive the same key. Without a path (a
//! virtual display without monitor device) the GDI name `\\.\DISPLAYn`
//! stands in. Rectangles are in the coordinates of the calling process:
//! physical pixels in a surface, scaled ones in the shell, where a changed
//! scale therefore shows as a changed rectangle (C1.2 DPI rule).
use sha2::{Digest, Sha256};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};

use crate::desktop::os::Monitor;

/// `MONITORINFOF_PRIMARY`.
const PRIMARY: u32 = 1;
/// `EDD_GET_DEVICE_INTERFACE_NAME`: `DeviceID` holds the interface path.
const DEVICE_INTERFACE_NAME: u32 = 1;
/// `DISPLAY_DEVICE_ACTIVE` of a monitor device.
const DEVICE_ACTIVE: u32 = 1;

/// One display monitor as the session reports it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MonitorInfo {
    pub(crate) key: String,
    /// Device interface path, the monitor ID of `IDesktopWallpaper`.
    pub(crate) device_path: Option<String>,
    /// GDI device name `\\.\DISPLAYn`.
    pub(crate) gdi_name: String,
    /// The monitor device's description (`DeviceString`).
    pub(crate) description: String,
    pub(crate) primary: bool,
    /// `rcMonitor` as `[left, top, right, bottom]`.
    pub(crate) rect: [i32; 4],
    /// `rcWork`: the monitor without task bar and docked bars.
    pub(crate) work: [i32; 4],
}

/// Key of a monitor: 16 hex characters of SHA-256 over its identity.
pub(crate) fn monitor_key(identity: &str) -> String {
    Sha256::digest(identity.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn text(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn corners(rect: RECT) -> [i32; 4] {
    [rect.left, rect.top, rect.right, rect.bottom]
}

/// Interface path and description of the active monitor on the GDI output
/// `gdi_name`; the first monitor device when none reports itself active.
fn monitor_device(gdi_name: &str) -> (Option<String>, String) {
    let name = wide(gdi_name);
    let mut first: Option<(Option<String>, String)> = None;
    for index in 0..16 {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        let found = unsafe {
            EnumDisplayDevicesW(
                PCWSTR(name.as_ptr()),
                index,
                &mut device,
                DEVICE_INTERFACE_NAME,
            )
        };
        if !found.as_bool() {
            break;
        }
        let path = Some(text(&device.DeviceID)).filter(|path| !path.is_empty());
        let entry = (path, text(&device.DeviceString));
        if device.StateFlags & DEVICE_ACTIVE != 0 {
            return entry;
        }
        first.get_or_insert(entry);
    }
    first.unwrap_or((None, String::new()))
}

/// Every monitor of the desktop in the order of `EnumDisplayMonitors`.
pub(crate) fn enumerate() -> Vec<MonitorInfo> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let handles = &mut *(data.0 as *mut Vec<HMONITOR>);
        handles.push(monitor);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC(std::ptr::null_mut()),
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }
    handles.into_iter().filter_map(info_of).collect()
}

fn info_of(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let read = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if !read.as_bool() {
        return None;
    }
    let gdi_name = text(&info.szDevice);
    let (device_path, description) = monitor_device(&gdi_name);
    let identity = device_path.clone().unwrap_or_else(|| gdi_name.clone());
    Some(MonitorInfo {
        key: monitor_key(&identity),
        device_path,
        description,
        primary: info.monitorInfo.dwFlags & PRIMARY != 0,
        rect: corners(info.monitorInfo.rcMonitor),
        work: corners(info.monitorInfo.rcWork),
        gdi_name,
    })
}

/// The monitors as the shell and the surfaces use them.
pub(crate) fn monitors() -> Vec<Monitor> {
    enumerate()
        .into_iter()
        .map(|info| Monitor {
            key: info.key,
            primary: info.primary,
            rect: Some(info.rect),
        })
        .collect()
}

/// The monitor with `key`, read now.
pub(crate) fn info_for(key: &str) -> Option<MonitorInfo> {
    enumerate().into_iter().find(|info| info.key == key)
}

/// `rcWork` of the monitor with `key`.
pub(crate) fn work_area(key: &str) -> Option<[i32; 4]> {
    info_for(key).map(|info| info.work)
}

/// The name of a monitor in menus: "Bildschirm <n>" after Windows'
/// `\\.\DISPLAY<n>`, with the monitor's description when it has one.
pub(crate) fn display_name(info: &MonitorInfo) -> String {
    let number: String = info
        .gdi_name
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    let base = if number.is_empty() {
        "Bildschirm".to_string()
    } else {
        format!("Bildschirm {number}")
    };
    match info.description.trim() {
        "" => base,
        description => format!("{base} – {description}"),
    }
}

/// `(key, name)` of every monitor ("Auf Bildschirm verschieben").
pub(crate) fn monitor_names() -> Vec<(String, String)> {
    enumerate()
        .iter()
        .map(|info| (info.key.clone(), display_name(info)))
        .collect()
}
