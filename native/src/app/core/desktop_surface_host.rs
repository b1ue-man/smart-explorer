//! The host of a surface view (DSK-M4): the view is written against
//! `egui::Context`/`egui::Ui` only, so the eframe window of one monitor and
//! the layer backend of DSK-M7 drive it alike. A host names its monitor and
//! the monitor's work area, knows every monitor of the session, hands out
//! its native window (DSK-M5 and DSK-M6 keep that window at the bottom) and
//! asks for frames.
use eframe::egui;
use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};

use crate::desktop::os::{platform, Monitor};

/// The native window of a surface.
// Staged: DSK-M5 (Windows styles, subclass) and DSK-M6 (EWMH) read the
// handles; DSK-M4 only hands them out.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeWindow {
    Win32 { hwnd: isize },
    Xlib { window: std::ffi::c_ulong },
    Xcb { window: u32 },
    Wayland,
    Other,
}

pub(crate) trait SurfaceHost {
    /// Key of this surface's monitor (`layout/<key>.json`).
    fn monitor_key(&self) -> &str;
    /// Every monitor of the session (`rect` in physical pixels).
    fn monitors(&self) -> &[Monitor];
    /// The monitor without task bars and panels, physical pixels.
    fn work_area(&self) -> Option<[i32; 4]>;
    /// The surface's native window.
    // Staged: called by the adapters of DSK-M5 and DSK-M6.
    #[allow(dead_code)]
    fn native_window(&self) -> Option<NativeWindow>;
    /// Asks for a new frame (a model arrived).
    fn request_repaint(&self);
}

/// This surface's monitor `[left, top, right, bottom]` in physical pixels.
pub(crate) fn monitor_rect(host: &dyn SurfaceHost) -> Option<[i32; 4]> {
    host.monitors()
        .iter()
        .find(|monitor| monitor.key == host.monitor_key())
        .and_then(|monitor| monitor.rect)
}

/// "Auf Bildschirm verschieben": the other monitors with a name.
pub(crate) fn other_monitors(host: &dyn SurfaceHost) -> Vec<(String, String)> {
    host.monitors()
        .iter()
        .enumerate()
        .filter(|(_, monitor)| monitor.key != host.monitor_key())
        .map(|(index, monitor)| {
            let name = if monitor.primary {
                format!("Bildschirm {} (Hauptbildschirm)", index + 1)
            } else {
                format!("Bildschirm {}", index + 1)
            };
            (monitor.key.clone(), name)
        })
        .collect()
}

/// The monitor under a point of the virtual desktop (physical pixels).
pub(crate) fn monitor_at(host: &dyn SurfaceHost, point: [i32; 2]) -> Option<String> {
    host.monitors()
        .iter()
        .find(|monitor| {
            monitor.rect.is_some_and(|[left, top, right, bottom]| {
                (left..right).contains(&point[0]) && (top..bottom).contains(&point[1])
            })
        })
        .map(|monitor| monitor.key.clone())
}

/// Every monitor together, physical pixels (wallpaper "Spannen").
pub(crate) fn virtual_desktop(host: &dyn SurfaceHost) -> Option<[i32; 4]> {
    host.monitors()
        .iter()
        .filter_map(|monitor| monitor.rect)
        .reduce(|all, rect| {
            [
                all[0].min(rect[0]),
                all[1].min(rect[1]),
                all[2].max(rect[2]),
                all[3].max(rect[3]),
            ]
        })
}

/// The host of an eframe window on one monitor.
pub(crate) struct EframeHost {
    ctx: egui::Context,
    monitor: String,
    monitors: Vec<Monitor>,
    window: Option<NativeWindow>,
}

impl EframeHost {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, monitor: &str) -> EframeHost {
        let window = cc
            .window_handle()
            .ok()
            .map(|handle| native_window_of(handle.as_raw()));
        EframeHost {
            ctx: cc.egui_ctx.clone(),
            monitor: monitor.to_string(),
            monitors: platform::monitors(),
            window,
        }
    }
}

fn native_window_of(raw: RawWindowHandle) -> NativeWindow {
    match raw {
        RawWindowHandle::Win32(handle) => NativeWindow::Win32 {
            hwnd: handle.hwnd.get(),
        },
        RawWindowHandle::Xlib(handle) => NativeWindow::Xlib {
            window: handle.window,
        },
        RawWindowHandle::Xcb(handle) => NativeWindow::Xcb {
            window: handle.window.get(),
        },
        RawWindowHandle::Wayland(_) => NativeWindow::Wayland,
        _ => NativeWindow::Other,
    }
}

impl SurfaceHost for EframeHost {
    fn monitor_key(&self) -> &str {
        &self.monitor
    }

    fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    /// DSK-M5 (`rcWork`) and DSK-M6 (`_NET_WORKAREA`) narrow it; until then
    /// the whole monitor.
    fn work_area(&self) -> Option<[i32; 4]> {
        monitor_rect(self)
    }

    fn native_window(&self) -> Option<NativeWindow> {
        self.window
    }

    fn request_repaint(&self) {
        self.ctx.request_repaint();
    }
}
