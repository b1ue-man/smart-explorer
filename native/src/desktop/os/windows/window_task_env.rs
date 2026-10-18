//! Environment of the DSK-M5 window and process checks
//! (`SMART_EXPLORER_DESKTOP_TASK=1`, one test thread, `--include-ignored`,
//! the GUI program from `SE_DESKTOP_TASK_GUI` or beside the test binary):
//! app data of this process and its children in a temporary folder
//! (`APPDATA`, `LOCALAPPDATA`, the test namespace of the instance mutex;
//! `USERPROFILE` stays, so known folders and the jump-list storage are the
//! runner's), surfaces and shells of the real GUI program, the windows of a
//! process, input through `SendInput`, a plain window of this process with
//! its message pump, and the Explorer precheck `FindWindowW("Progman")`
//! (Annahme A7).
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, FindWindowW,
    GetAncestor, GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, PeekMessageW, RegisterClassW, ShowWindow,
    TranslateMessage, WindowFromPoint, GA_ROOT, GWL_EXSTYLE, GWL_STYLE, GW_HWNDNEXT, MSG,
    PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
};

use crate::desktop::core::protocol::{read_frame, SurfaceMessage};
use crate::desktop::core::state::{ModuleState, Part, PartSet, State, STATE_SCHEMA};
use crate::desktop::os::store::DesktopDir;

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(super) fn gui_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("SE_DESKTOP_TASK_GUI") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap();
    let target = exe.parent().and_then(Path::parent).unwrap();
    let gui = target.join("smart_explorer.exe");
    assert!(gui.is_file(), "{} fehlt", gui.display());
    gui
}

pub(super) fn wait_for(limit: Duration, what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "{what}: nach {limit:?} nicht erreicht"
        );
        pump(Duration::from_millis(50));
    }
}

/// Dispatches this thread's messages for `time` (the test window needs it;
/// activation messages of other threads wait for it).
pub(super) fn pump(time: Duration) {
    let deadline = Instant::now() + time;
    loop {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Explorer's desktop window exists in this session (Annahme A7).
pub(super) fn progman_present() -> bool {
    let class = wide("Progman");
    !unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) }.is_null()
}

/// The visible top-level windows of process `pid`.
pub(super) fn windows_of(pid: u32) -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        let (pid, found) = &mut *(data as *mut (u32, Vec<HWND>));
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, &mut owner);
        if owner == *pid && IsWindowVisible(hwnd) != 0 {
            found.push(hwnd);
        }
        1
    }
    let mut data: (u32, Vec<HWND>) = (pid, Vec::new());
    unsafe { EnumWindows(Some(collect), &mut data as *mut (u32, Vec<HWND>) as LPARAM) };
    data.1
}

pub(super) fn ex_style(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }
}

pub(super) fn style(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 }
}

pub(super) fn rect(hwnd: HWND) -> [i32; 4] {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut rect) };
    [rect.left, rect.top, rect.right, rect.bottom]
}

/// The surface window of `pid` once it is a tool window.
pub(super) fn surface_window(pid: u32) -> HWND {
    let mut found = std::ptr::null_mut();
    wait_for(Duration::from_secs(30), "Flächenfenster", || {
        found = windows_of(pid)
            .into_iter()
            .find(|hwnd| ex_style(*hwnd) & WS_EX_TOOLWINDOW != 0)
            .unwrap_or(std::ptr::null_mut());
        !found.is_null()
    });
    found
}

/// Whether `lower` comes after `upper` in the z-order.
pub(super) fn below(upper: HWND, lower: HWND) -> bool {
    let mut next = unsafe { GetWindow(upper, GW_HWNDNEXT) };
    while !next.is_null() {
        if next == lower {
            return true;
        }
        next = unsafe { GetWindow(next, GW_HWNDNEXT) };
    }
    false
}

/// The top-level window under a point of the screen.
pub(super) fn root_at(point: [i32; 2]) -> HWND {
    let hit = unsafe {
        WindowFromPoint(POINT {
            x: point[0],
            y: point[1],
        })
    };
    if hit.is_null() {
        return hit;
    }
    unsafe { GetAncestor(hit, GA_ROOT) }
}

fn send(inputs: &[INPUT]) {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    assert_eq!(sent as usize, inputs.len(), "SendInput");
}

fn key(code: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Presses `keys` in order and releases them in reverse order.
pub(super) fn chord(keys: &[u16]) {
    let mut inputs: Vec<INPUT> = keys.iter().map(|code| key(*code, false)).collect();
    inputs.extend(keys.iter().rev().map(|code| key(*code, true)));
    send(&inputs);
}

/// A left click at a point of the screen.
pub(super) fn click(point: [i32; 2]) {
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    };
    let scale =
        |value: i32, origin: i32, size: i32| ((value - origin) * 65_535) / (size - 1).max(1);
    let mouse = |flags: u32| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: scale(point[0], left, width),
                dy: scale(point[1], top, height),
                mouseData: 0,
                dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(&[
        mouse(MOUSEEVENTF_MOVE),
        mouse(MOUSEEVENTF_LEFTDOWN),
        mouse(MOUSEEVENTF_LEFTUP),
    ]);
}

unsafe extern "system" fn plain_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, message, wparam, lparam)
}

/// An ordinary top-level window of this process ("ein Programmfenster").
pub(super) struct PlainWindow(pub(super) HWND);

impl PlainWindow {
    pub(super) fn new(at: [i32; 4]) -> PlainWindow {
        let class = wide("SeDesktopTaskWindow");
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let mut definition: WNDCLASSW = std::mem::zeroed();
            definition.lpfnWndProc = Some(plain_window_proc);
            definition.hInstance = instance;
            definition.lpszClassName = class.as_ptr();
            // A second registration in the same process fails harmlessly.
            RegisterClassW(&definition);
            let title = wide("se Desktop Testfenster");
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                at[0],
                at[1],
                at[2] - at[0],
                at[3] - at[1],
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            assert!(!hwnd.is_null(), "Testfenster");
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            pump(Duration::from_millis(200));
            PlainWindow(hwnd)
        }
    }
}

impl Drop for PlainWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) };
        pump(Duration::from_millis(50));
    }
}

/// Isolated app data of this process and its children; ends what it
/// started.
pub(super) struct WinTaskEnv {
    pub(super) gui: PathBuf,
    saved: Vec<(String, Option<OsString>)>,
    children: Vec<Child>,
    _root: tempfile::TempDir,
}

impl WinTaskEnv {
    pub(super) fn new() -> WinTaskEnv {
        assert_eq!(
            std::env::var("SMART_EXPLORER_DESKTOP_TASK").as_deref(),
            Ok("1"),
            "isolated remote task entrypoint required"
        );
        let gui = gui_binary();
        let root = tempfile::tempdir().unwrap();
        let vars: Vec<(&str, OsString)> = vec![
            ("APPDATA", root.path().join("appdata").into()),
            ("LOCALAPPDATA", root.path().join("local").into()),
            (
                "SMART_EXPLORER_E2E_TEST_NAMESPACE",
                format!("desktop_task_win_{}", std::process::id()).into(),
            ),
        ];
        let mut saved = Vec::new();
        for (name, value) in vars {
            saved.push((name.to_string(), std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        WinTaskEnv {
            gui,
            saved,
            children: Vec::new(),
            _root: root,
        }
    }

    pub(super) fn dir(&self) -> DesktopDir {
        DesktopDir::production()
    }

    /// `state.json` enabled with `parts`.
    pub(super) fn enable(&self, parts: &[Part]) {
        let dir = self.dir();
        dir.create().unwrap();
        dir.write_state(&ModuleState {
            schema: STATE_SCHEMA,
            state: State::Enabled,
            parts: PartSet::of(parts),
            changed_at: 1,
            halted: None,
            ledger: Vec::new(),
        })
        .unwrap();
    }

    pub(super) fn spawn(&mut self, args: &[&str]) -> usize {
        let child = Command::new(&self.gui)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.children.push(child);
        self.children.len() - 1
    }

    /// A surface on `monitor` whose channel stays open; its output is read
    /// and dropped in a thread.
    pub(super) fn surface(&mut self, monitor: &str) -> usize {
        let mut child = Command::new(&self.gui)
            .args(["--desktop-surface", "--monitor", monitor])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut output = child.stdout.take().unwrap();
        let hello: Option<SurfaceMessage> = read_frame(&mut output).unwrap();
        assert!(
            matches!(&hello, Some(SurfaceMessage::Hello { monitor: seen, .. }) if seen == monitor),
            "{hello:?}"
        );
        std::thread::spawn(move || {
            let mut sink = [0u8; 4096];
            while output.read(&mut sink).is_ok_and(|read| read > 0) {}
        });
        self.children.push(child);
        self.children.len() - 1
    }

    pub(super) fn child(&mut self, index: usize) -> &mut Child {
        &mut self.children[index]
    }

    /// `shell.log` events named `name` (details).
    pub(super) fn events(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.dir().log_path())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let _at = parts.next()?;
                (parts.next()? == name).then(|| parts.next().unwrap_or_default().to_string())
            })
            .collect()
    }
}

impl Drop for WinTaskEnv {
    fn drop(&mut self) {
        let dir = self.dir();
        if dir.exists() {
            let _ = dir.request_stop();
        }
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        std::thread::sleep(Duration::from_millis(1500));
        for (name, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(&name, value),
                None => std::env::remove_var(&name),
            }
        }
    }
}
