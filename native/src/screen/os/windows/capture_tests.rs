//! Task tests of the Windows screen host (SCR-M4, stage `windows-screen`)
//! with real windows and monitors, in this process: a test window of 1280 ×
//! 720 pixels (smaller where the primary monitor is: a part of a window that
//! lies outside the desktop is never painted, so the window must fit on it)
//! that a timer repaints every 16 ms (a moving rectangle on a
//! known background), the window list, the state of a minimized window,
//! capture through the backend with the tile encoder, the DXGI fallback
//! and the lock screen. Tests that need a desktop are `#[ignore]` and run
//! with `--include-ignored` and `SE_SCREEN_TASK_WINDOWS=1` on a runner with
//! an interactive session; without the variable they report that they did
//! not run.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, GetDC, ReleaseDC, HBRUSH, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetTimer, ShowWindow, TranslateMessage, MSG, SW_MINIMIZE, SW_RESTORE,
    WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_POPUP, WS_VISIBLE,
};

use super::host_backend::WindowsBackend;
use super::{capture_wgc, window_list};
use crate::screen::core::host_backend::{ScreenHostBackend, ScreenSource, SourceFrame};
use crate::screen::core::negotiate::{CodecChoice, CodecKind};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::{ListSource, SourceId};

/// Largest size of the test window; `window_size` shrinks it to the primary
/// monitor.
const MAX_WIDTH: i32 = 1280;
const MAX_HEIGHT: i32 = 720;
/// Background of the test window as BGR bytes.
const BACKGROUND: [u8; 3] = [0x80, 0x40, 0x20];

static FRAME: AtomicU32 = AtomicU32::new(0);

/// Size of the primary monitor in pixels as the process sees it.
fn screen_size() -> (i32, i32) {
    // SAFETY: plain metric queries.
    unsafe {
        (
            windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows_sys::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
            ),
            windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows_sys::Win32::UI::WindowsAndMessaging::SM_CYSCREEN,
            ),
        )
    }
}

/// The size of the test window: 1280 × 720, or the primary monitor's size
/// where that is smaller. The window sits at the desktop's origin; a hosted
/// runner has a display of 1024 × 768, where a window of 1280 × 720 at
/// (50, 50) would reach beyond the desktop and GDI never paints that part
/// (the capture then shows black there).
fn window_size() -> (i32, i32) {
    static SIZE: OnceLock<(i32, i32)> = OnceLock::new();
    *SIZE.get_or_init(|| {
        let (screen_width, screen_height) = screen_size();
        let fit = |wanted: i32, screen: i32| {
            if screen > 0 {
                wanted.min(screen)
            } else {
                wanted
            }
        };
        (fit(MAX_WIDTH, screen_width), fit(MAX_HEIGHT, screen_height))
    })
}

/// Whether the task suite runs the tests with real windows here.
fn desktop_tests() -> bool {
    let enabled = std::env::var("SE_SCREEN_TASK_WINDOWS").as_deref() == Ok("1");
    if !enabled {
        eprintln!("not run: SE_SCREEN_TASK_WINDOWS=1 is not set");
    }
    enabled
}

fn colorref(blue: u8, green: u8, red: u8) -> COLORREF {
    COLORREF(u32::from(red) | u32::from(green) << 8 | u32::from(blue) << 16)
}

fn paint(hwnd: HWND) {
    let frame = FRAME.fetch_add(1, Ordering::SeqCst);
    let (width, height) = window_size();
    let left = 100 + (frame as i32 * 9) % 1000;
    let background = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let moving = RECT {
        left,
        top: 300,
        right: left + 120,
        bottom: 420,
    };
    // SAFETY: the device context and both brushes are released and deleted
    // before the function returns.
    unsafe {
        let dc: HDC = GetDC(hwnd);
        let base: HBRUSH = CreateSolidBrush(colorref(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]));
        let mark: HBRUSH = CreateSolidBrush(colorref(0x20, 0xC0, (frame % 256) as u8));
        FillRect(dc, &background, base);
        FillRect(dc, &moving, mark);
        let _ = DeleteObject(HGDIOBJ(base.0));
        let _ = DeleteObject(HGDIOBJ(mark.0));
        ReleaseDC(hwnd, dc);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends the message loop of this window's thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: the default procedure for every message not handled.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// A window of the test with its message loop on a thread of its own.
struct TestWindow {
    handle: usize,
    thread: Option<JoinHandle<()>>,
}

impl TestWindow {
    fn open(title: &str) -> Self {
        let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            // SAFETY: a window class and a window created and used by this
            // thread alone; the loop ends with `WM_DESTROY`.
            unsafe {
                let module =
                    windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
                let instance = HINSTANCE(module);
                let (width, height) = window_size();
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance,
                    lpszClassName: w!("SmartExplorerScreenTask"),
                    ..Default::default()
                };
                // A second window finds the class registered already.
                let _ = RegisterClassW(&class);
                let created = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("SmartExplorerScreenTask"),
                    PCWSTR(title.as_ptr()),
                    WS_POPUP | WS_VISIBLE,
                    0,
                    0,
                    width,
                    height,
                    HWND(std::ptr::null_mut()),
                    windows::Win32::UI::WindowsAndMessaging::HMENU(std::ptr::null_mut()),
                    instance,
                    None,
                );
                let Ok(hwnd) = created else {
                    let _ = sender.send(0usize);
                    return;
                };
                SetTimer(hwnd, 1, 16, None);
                let _ = sender.send(hwnd.0 as usize);
                let mut message = MSG::default();
                while GetMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        });
        let handle = receiver.recv().unwrap();
        assert_ne!(handle, 0, "the test window was not created");
        // The first repaint and the compositor need a moment.
        std::thread::sleep(Duration::from_millis(300));
        Self {
            handle,
            thread: Some(thread),
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.handle as *mut _)
    }

    fn source_id(&self) -> SourceId {
        SourceId::Window(self.handle.to_string())
    }

    fn show(&self, command: windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD) {
        // SAFETY: a plain call on the test window's handle.
        unsafe {
            let _ = ShowWindow(self.hwnd(), command);
        }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: asks the window to close; its thread then ends.
        unsafe {
            let _ = PostMessageW(self.hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn unique_title() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("SmartExplorer ScreenTask {}-{nanos}", std::process::id())
}

/// The next picture of `source`, waiting up to `limit` through pauses that
/// say "nothing yet".
fn next_picture(source: &mut dyn ScreenSource, limit: Duration) -> BgraImage {
    let deadline = Instant::now() + limit;
    loop {
        match source.capture(1, true).expect("capture failed") {
            SourceFrame::Frame(image) => return image,
            SourceFrame::Unchanged | SourceFrame::Paused(SessionState::Starting) => {}
            SourceFrame::Paused(state) => panic!("capture paused: {}", state.key()),
        }
        assert!(Instant::now() < deadline, "no picture within {limit:?}");
    }
}

fn pixel(image: &BgraImage, x: u32, y: u32) -> [u8; 3] {
    let row = image.view().row(y);
    let at = x as usize * 4;
    [row[at], row[at + 1], row[at + 2]]
}

#[test]
#[ignore = "needs an interactive Windows session; SE_SCREEN_TASK_WINDOWS=1"]
fn screen_task_windows_wgc_snapshot_matches() {
    if !desktop_tests() || !capture_wgc::available() {
        return;
    }
    let window = TestWindow::open(&unique_title());
    let mut source = WindowsBackend::new()
        .open_source(&window.source_id(), None)
        .expect("the test window opens");
    let (width, height) = window_size();
    let (width, height) = (width as u32, height as u32);
    let size = FrameSize::new(width, height);
    assert_eq!(source.size(), size, "screen {:?}", screen_size());
    let image = next_picture(source.as_mut(), Duration::from_secs(5));
    assert_eq!(image.size(), size, "screen {:?}", screen_size());
    // The corners lie outside the moving rectangle: they show what the
    // window drew, and the alpha channel is opaque.
    for (x, y) in [(5, 5), (width - 5, 5), (5, height - 5)] {
        assert_eq!(
            pixel(&image, x, y),
            BACKGROUND,
            "pixel {x},{y} of {width}x{height}, screen {:?}",
            screen_size()
        );
    }
    assert!(image.data().chunks_exact(4).all(|pixel| pixel[3] == 255));
    // The still picture of it is a PNG of the same size.
    let png = crate::screen::core::snapshot::encode_png(&image).unwrap();
    assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
}

#[test]
#[ignore = "needs an interactive Windows session; SE_SCREEN_TASK_WINDOWS=1"]
fn screen_task_windows_window_list_has_test_window() {
    if !desktop_tests() {
        return;
    }
    let title = unique_title();
    let window = TestWindow::open(&title);
    let list = WindowsBackend::new().windows().unwrap();
    assert_eq!(list.list_source, ListSource::EnumWindows);
    assert!(list.apps.is_empty());
    assert!(!list.limits.is_empty());
    let entry = list
        .windows
        .iter()
        .find(|entry| entry.title == title)
        .expect("the test window is listed");
    assert_eq!(entry.id, window.handle.to_string());
    assert_eq!(entry.pid, Some(std::process::id()));
    assert!(!entry.app.is_empty());
    assert!(!entry.minimized);
    assert_eq!(window_list::window_id(window.hwnd()), entry.id);
    let monitors = WindowsBackend::new().monitors();
    assert!(entry
        .monitor
        .as_ref()
        .is_some_and(|id| monitors.iter().any(|monitor| &monitor.id == id)));
}

#[test]
#[ignore = "needs an interactive Windows session; SE_SCREEN_TASK_WINDOWS=1"]
fn screen_task_windows_minimized_state() {
    if !desktop_tests() || !capture_wgc::available() {
        return;
    }
    let title = unique_title();
    let window = TestWindow::open(&title);
    let backend = WindowsBackend::new();
    let mut source = backend.open_source(&window.source_id(), None).unwrap();
    next_picture(source.as_mut(), Duration::from_secs(5));
    window.show(SW_MINIMIZE);
    std::thread::sleep(Duration::from_millis(500));
    assert!(matches!(
        source.capture(2, false).unwrap(),
        SourceFrame::Paused(SessionState::WindowMinimized)
    ));
    let listed = backend.windows().unwrap();
    // A minimized window may leave the list on some builds; when it stays,
    // it says so.
    if let Some(entry) = listed.windows.iter().find(|entry| entry.title == title) {
        assert!(entry.minimized);
    }
    window.show(SW_RESTORE);
    std::thread::sleep(Duration::from_millis(500));
    next_picture(source.as_mut(), Duration::from_secs(5));
}

#[test]
#[ignore = "needs an interactive Windows session; SE_SCREEN_TASK_WINDOWS=1"]
fn screen_task_windows_bench_real_capture() {
    if !desktop_tests() || !capture_wgc::available() {
        return;
    }
    let window = TestWindow::open(&unique_title());
    let backend = WindowsBackend::new();
    let mut source = backend.open_source(&window.source_id(), None).unwrap();
    let first = next_picture(source.as_mut(), Duration::from_secs(5));
    let choice = CodecChoice {
            max_macroblocks_per_second: None,
        codec: CodecKind::Tiles,
        profile: None,
        encoder_hardware: false,
        decoder_hardware: false,
        max_size: FrameSize::MAX,
        max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
    };
    let mut encoder = crate::screen::os::codec_inventory::registry()
        .open_encoder(&choice, first.size(), 60)
        .expect("the tile encoder opens")
        .encoder;
    let mut distinct = std::collections::HashSet::new();
    let mut encoded = 0usize;
    let end = Instant::now() + Duration::from_secs(2);
    let mut frame_id = 1;
    while Instant::now() < end {
        frame_id += 1;
        // The threshold of 15 distinct pictures in 2 s is assumption A1 of
        // the strand and is calibrated after the first measurement.
        match source.capture(frame_id, false).expect("capture failed") {
            SourceFrame::Frame(image) => {
                let mut hasher = DefaultHasher::new();
                image.data().hash(&mut hasher);
                distinct.insert(hasher.finish());
                let frame = encoder.encode(&image, encoded == 0, 0).unwrap();
                encoded += usize::from(!frame.records.is_empty());
            }
            SourceFrame::Unchanged | SourceFrame::Paused(SessionState::Starting) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            SourceFrame::Paused(state) => panic!("capture paused: {}", state.key()),
        }
    }
    assert!(distinct.len() >= 15, "{} distinct pictures", distinct.len());
    assert!(encoded >= 15, "{encoded} encoded pictures");
}

#[test]
#[ignore = "needs an interactive Windows session; SE_SCREEN_TASK_WINDOWS=1 and SE_SCREEN_TASK_DXGI=1"]
fn screen_task_windows_dxgi_fallback_captures_monitor() {
    if !desktop_tests() || std::env::var("SE_SCREEN_TASK_DXGI").as_deref() != Ok("1") {
        eprintln!("not run: SE_SCREEN_TASK_DXGI=1 is not set");
        return;
    }
    capture_wgc::force_unavailable_for_tests(true);
    let backend = WindowsBackend::new();
    let monitors = backend.monitors();
    let primary = monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or(monitors.first())
        .expect("the session has a monitor");
    assert!(backend.limits().contains(&"no_cursor_capture".to_string()));
    let mut source = backend
        .open_source(&SourceId::Monitor(primary.id.clone()), None)
        .expect("the monitor opens with the duplication");
    let image = next_picture(source.as_mut(), Duration::from_secs(5));
    capture_wgc::force_unavailable_for_tests(false);
    let expected = FrameSize::new(primary.width, primary.height).fit_within(FrameSize::MAX);
    assert_eq!(image.size(), expected);
}

#[test]
fn screen_task_windows_synthetic_source_still_works() {
    let backend = WindowsBackend::new();
    let mut source = backend
        .open_source(&SourceId::Synthetic, Some(FrameSize::new(640, 360)))
        .expect("the test picture opens on Windows");
    assert_eq!(source.size(), FrameSize::new(640, 360));
    let SourceFrame::Frame(first) = source.capture(1, false).unwrap() else {
        panic!("the test picture gives a picture");
    };
    let SourceFrame::Frame(second) = source.capture(2, false).unwrap() else {
        panic!("the test picture gives a picture");
    };
    assert_ne!(first, second);
    assert_eq!(
        backend.platform(),
        crate::screen::core::wire_hello::HostPlatform::Windows
    );
    assert_eq!(backend.encoders().len(), 1);
}

#[test]
fn screen_task_windows_monitor_ids_stable() {
    let first = WindowsBackend::new().monitors();
    let second = WindowsBackend::new().monitors();
    assert_eq!(first, second);
    let mut ids: Vec<&String> = first.iter().map(|monitor| &monitor.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), first.len(), "monitor ids are unique");
    assert!(first.iter().all(|monitor| !monitor.id.is_empty()
        && monitor.width > 0
        && monitor.height > 0
        && monitor.name.starts_with("Bildschirm")));
    if !first.is_empty() {
        assert_eq!(first.iter().filter(|monitor| monitor.primary).count(), 1);
    }
}

#[link(name = "user32")]
extern "system" {
    fn LockWorkStation() -> i32;
}

/// The last step of the Windows job: locks the workstation and expects the
/// secure desktop within 10 seconds.
#[test]
#[ignore = "locks the workstation; last step of the windows-screen stage; SE_SCREEN_TASK_WINDOWS=1"]
fn screen_task_windows_lock_reports_secure_desktop() {
    if !desktop_tests() {
        return;
    }
    assert!(!super::secure_desktop::on_secure_desktop());
    // SAFETY: a call without arguments.
    assert_ne!(unsafe { LockWorkStation() }, 0, "LockWorkStation failed");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !super::secure_desktop::on_secure_desktop() {
        assert!(
            Instant::now() < deadline,
            "the lock screen was not reported as the secure desktop within 10 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
