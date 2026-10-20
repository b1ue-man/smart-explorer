//! The host of a surface view (DSK-M4): the view is written against
//! `egui::Context`/`egui::Ui` only, so the eframe window of one monitor and
//! the layer backend of DSK-M7 drive it alike. A host names its monitor and
//! the monitor's work area, knows every monitor of the session, hands out
//! its native window and asks for frames.
//!
//! DSK-M5: the eframe host hands its window to the platform right after
//! creation (Windows: lowest tool window with subclass, `surface_window`),
//! keeps the work area current (`rcWork`), names the monitors, reads the
//! system wallpaper and runs Explorer actions. What the native window
//! notices arrives as [`SurfaceSignals`]; a native Ctrl+V without text on
//! the clipboard (a file copy in Explorer) becomes egui's paste event at the
//! start of the next pass, so the view's paste handling takes it.
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};

use super::desktop_surface_clipboard::FileClipboard;
use crate::desktop::os::wallpaper_image::SystemWallpaper;
use crate::desktop::os::{platform, Monitor};

/// The suite's window test mode (DSK-M15): an ordinary window, untouched.
const TEST_WINDOWED: &str = "SE_DESKTOP_TEST_WINDOWED";
pub(crate) const NO_SYSTEM_BACKGROUND: &str = "Systemhintergrund hier nicht lesbar";
const NO_SHELL_ACTIONS: &str = "Explorer-Aktionen gibt es nur unter Windows";

/// The native window of a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeWindow {
    Win32 { hwnd: isize },
    Xlib { window: std::ffi::c_ulong },
    Xcb { window: u32 },
    Wayland,
    Other,
}

impl NativeWindow {
    /// The window's handle or X11 ID, for messages.
    pub(crate) fn id(&self) -> Option<u64> {
        match self {
            NativeWindow::Win32 { hwnd } => Some(*hwnd as u64),
            NativeWindow::Xlib { window } => Some(u64::from(*window)),
            NativeWindow::Xcb { window } => Some(u64::from(*window)),
            NativeWindow::Wayland | NativeWindow::Other => None,
        }
    }
}

/// What the native window of a surface noticed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceSignal {
    /// Ctrl+V or Shift+Insert while the clipboard holds no text.
    Paste,
    /// Monitors, their arrangement or the work area changed.
    Display,
    /// The system's wallpaper changed.
    Wallpaper,
}

/// Raised by the window procedure, taken by the view; every raise asks for
/// a frame.
pub(crate) struct SurfaceSignals {
    paste: AtomicBool,
    display: AtomicBool,
    wallpaper: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl SurfaceSignals {
    pub(crate) fn new(wake: impl Fn() + Send + Sync + 'static) -> Arc<SurfaceSignals> {
        Arc::new(SurfaceSignals {
            paste: AtomicBool::new(false),
            display: AtomicBool::new(false),
            wallpaper: AtomicBool::new(false),
            wake: Box::new(wake),
        })
    }

    fn flag(&self, signal: SurfaceSignal) -> &AtomicBool {
        match signal {
            SurfaceSignal::Paste => &self.paste,
            SurfaceSignal::Display => &self.display,
            SurfaceSignal::Wallpaper => &self.wallpaper,
        }
    }

    /// The Windows window procedure and the X11 watch thread raise.
    pub(crate) fn raise(&self, signal: SurfaceSignal) {
        self.flag(signal).store(true, Ordering::Release);
        (self.wake)();
    }

    /// Whether `signal` was raised since the last call.
    pub(crate) fn take(&self, signal: SurfaceSignal) -> bool {
        self.flag(signal).swap(false, Ordering::AcqRel)
    }
}

/// The system wallpaper of one monitor (DSK-M5; Linux with DSK-M6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemBackground {
    pub(crate) wallpaper: SystemWallpaper,
    /// The system changes the image itself this often (slideshow).
    pub(crate) refresh_every: Option<Duration>,
}

/// An Explorer action of the surface (Windows).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ShellAction<'a> {
    /// "Weitere Optionen anzeigen" on desktop files.
    ItemMenu(&'a [PathBuf]),
    /// "Weitere Optionen anzeigen" on the free surface (its folder).
    FolderMenu(&'a Path),
    /// Desktop files dragged out to another program.
    DragOut(&'a [PathBuf]),
}

/// How an Explorer action ended (Windows). Staged on Linux, which has no
/// Explorer actions and never builds a value (`platform::shell_action`).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellOutcome {
    /// A command ran or the files were dropped.
    Done,
    /// The menu closed or the drag ended without a choice.
    Nothing,
}

pub(crate) trait SurfaceHost {
    /// Key of this surface's monitor (`layout/<key>.json`).
    fn monitor_key(&self) -> &str;
    /// Every monitor of the session (`rect` in physical pixels; in a layer
    /// session, DSK-M7, in the compositor's logical coordinates).
    fn monitors(&self) -> &[Monitor];
    /// The monitor without task bars and panels, physical pixels.
    fn work_area(&self) -> Option<[i32; 4]>;
    /// The surface's native window.
    fn native_window(&self) -> Option<NativeWindow>;
    /// Asks for a new frame (a model arrived).
    fn request_repaint(&self);
    /// The platform's name of a monitor, for menus.
    fn monitor_name(&self, _key: &str) -> Option<String> {
        None
    }
    /// The system background of this surface's monitor, read now.
    fn system_background(&self) -> Result<SystemBackground, String> {
        Err(NO_SYSTEM_BACKGROUND.into())
    }
    /// Whether the system's wallpaper changed since the last call. Linux:
    /// the poll of the wallpaper source (a layer host asks it too, DSK-M7).
    fn take_wallpaper_change(&self) -> bool {
        polled_wallpaper_change(self.monitor_key())
    }
    /// Runs an Explorer action; `heartbeat` keeps the surface alive while
    /// the action's modal loop runs.
    fn shell_action(
        &self,
        _action: ShellAction<'_>,
        _heartbeat: &mut dyn FnMut(),
    ) -> Result<ShellOutcome, String> {
        Err(NO_SHELL_ACTIONS.into())
    }
    /// The pointer is over another program's window (a drag may go out).
    fn pointer_outside(&self) -> bool {
        false
    }
    /// The point of the virtual desktop, in the coordinates of
    /// [`Monitor::rect`], under `pointer` (points of this surface, also
    /// outside it while a button is held). `None` lets the view work it out
    /// from the window's `inner_rect` (Windows, X11); the layer backend has
    /// no such rectangle and answers itself (DSK-M7).
    fn global_point(&self, _pointer: [f32; 2]) -> Option<[i32; 2]> {
        None
    }
    /// The file clipboard this host reads and writes through, when it has
    /// one of its own. The actions reach the same provider through
    /// `desktop_surface_clipboard::installed()`.
    #[allow(dead_code)]
    fn file_clipboard(&self) -> Option<Arc<dyn FileClipboard>> {
        None
    }
    /// Hands the surface the button release a drag loop took.
    fn release_pointer(&self) {}
}

/// The poll of the Linux wallpaper source (`platform::wallpaper_changed`);
/// Windows tells by signal.
fn polled_wallpaper_change(monitor: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        platform::wallpaper_changed(monitor)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = monitor;
        false
    }
}

/// This surface's monitor `[left, top, right, bottom]` in physical pixels.
pub(crate) fn monitor_rect(host: &dyn SurfaceHost) -> Option<[i32; 4]> {
    host.monitors()
        .iter()
        .find(|monitor| monitor.key == host.monitor_key())
        .and_then(|monitor| monitor.rect)
}

/// Where `other` lies seen from `this`, when it lies clearly beside it.
fn direction(this: [i32; 4], other: [i32; 4]) -> Option<&'static str> {
    if other[0] >= this[2] {
        Some("rechts")
    } else if other[2] <= this[0] {
        Some("links")
    } else if other[1] >= this[3] {
        Some("unten")
    } else if other[3] <= this[1] {
        Some("oben")
    } else {
        None
    }
}

/// "Auf Bildschirm verschieben": the other monitors with a name — the
/// platform's (Windows: number and model), else the number in the list —
/// and where they lie.
pub(crate) fn other_monitors(host: &dyn SurfaceHost) -> Vec<(String, String)> {
    let this = monitor_rect(host);
    host.monitors()
        .iter()
        .enumerate()
        .filter(|(_, monitor)| monitor.key != host.monitor_key())
        .map(|(index, monitor)| {
            let mut name = host
                .monitor_name(&monitor.key)
                .unwrap_or_else(|| format!("Bildschirm {}", index + 1));
            let side = this
                .zip(monitor.rect)
                .and_then(|(this, other)| direction(this, other));
            match (side, monitor.primary) {
                (Some(side), true) => name.push_str(&format!(" ({side}, Hauptbildschirm)")),
                (Some(side), false) => name.push_str(&format!(" ({side})")),
                (None, true) => name.push_str(" (Hauptbildschirm)"),
                (None, false) => {}
            }
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

/// The host of an eframe window on one monitor. Its monitors are read once
/// after the window exists (physical pixels); a changed monitor list makes
/// the shell start every surface anew. The work area follows the platform's
/// change notices.
pub(crate) struct EframeHost {
    ctx: egui::Context,
    monitor: String,
    monitors: Vec<Monitor>,
    names: Vec<(String, String)>,
    work: Cell<Option<[i32; 4]>>,
    window: Option<NativeWindow>,
    signals: Arc<SurfaceSignals>,
}

impl EframeHost {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, monitor: &str) -> EframeHost {
        let window = cc
            .window_handle()
            .ok()
            .map(|handle| native_window_of(handle.as_raw()));
        let wake = cc.egui_ctx.clone();
        let signals = SurfaceSignals::new(move || wake.request_repaint());
        let windowed = std::env::var(TEST_WINDOWED).is_ok_and(|value| value == "1");
        if let Some(window) = window.filter(|_| !windowed) {
            if let Err(error) = platform::attach_surface(window, monitor, signals.clone()) {
                eprintln!("se Desktop: Fenster {:?}: {error}", window.id());
            }
        }
        let paste = signals.clone();
        cc.egui_ctx.on_begin_pass(
            "se-desktop-native-paste",
            Arc::new(move |ctx| {
                if paste.take(SurfaceSignal::Paste) {
                    ctx.input_mut(|input| input.events.push(egui::Event::Paste(String::new())));
                }
            }),
        );
        EframeHost {
            ctx: cc.egui_ctx.clone(),
            monitor: monitor.to_string(),
            monitors: platform::monitors(),
            names: platform::monitor_names(),
            work: Cell::new(platform::work_area(monitor)),
            window,
            signals,
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

    /// The platform's work area (Windows `rcWork`), read again after a
    /// change notice; the whole monitor while the platform names none.
    fn work_area(&self) -> Option<[i32; 4]> {
        if self.signals.take(SurfaceSignal::Display) {
            self.work.set(platform::work_area(&self.monitor));
        }
        self.work.get().or_else(|| monitor_rect(self))
    }

    fn native_window(&self) -> Option<NativeWindow> {
        self.window
    }

    fn request_repaint(&self) {
        self.ctx.request_repaint();
    }

    fn monitor_name(&self, key: &str) -> Option<String> {
        self.names
            .iter()
            .find(|(known, _)| known == key)
            .map(|(_, name)| name.clone())
    }

    fn system_background(&self) -> Result<SystemBackground, String> {
        platform::system_wallpaper(&self.monitor)
    }

    fn take_wallpaper_change(&self) -> bool {
        self.signals.take(SurfaceSignal::Wallpaper) | polled_wallpaper_change(&self.monitor)
    }

    fn shell_action(
        &self,
        action: ShellAction<'_>,
        heartbeat: &mut dyn FnMut(),
    ) -> Result<ShellOutcome, String> {
        platform::shell_action(action, heartbeat)
    }

    fn pointer_outside(&self) -> bool {
        self.window.is_some_and(platform::pointer_outside)
    }

    fn release_pointer(&self) {
        if let Some(window) = self.window {
            platform::release_pointer(window);
        }
    }
}
