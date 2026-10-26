//! Broadcast receiver with its own hidden top-level window and a real message loop.
use crate::app::drive_events::Invalidate;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

struct State {
    invalidate: Invalidate,
    stop: Arc<AtomicBool>,
}

pub(crate) fn start_drive_events(invalidate: Invalidate, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("drive-broadcast".into())
        .spawn(move || {
            if let Err(error) = run(invalidate, stop) {
                // Inventory polling and focus refresh continue if registration fails.
                eprintln!("Laufwerksereignisse: {error}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("Laufwerksereignisse: {error}");
    }
}

fn run(invalidate: Invalidate, stop: Arc<AtomicBool>) -> Result<(), String> {
    let class: Vec<u16> = "SmartExplorerDriveEvents\0".encode_utf16().collect();
    unsafe {
        let instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        // A second app window may already have registered this class in this process.
        if RegisterClassW(&wc) == 0 && windows_sys::Win32::Foundation::GetLastError() != 1410 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut state = Box::new(State { invalidate, stop });
        // No HWND_MESSAGE: volume broadcasts target top-level windows.
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            (&mut *state as *mut State).cast(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if SetTimer(hwnd, 1, 500, None) == 0 {
            DestroyWindow(hwnd);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut message: MSG = std::mem::zeroed();
        let outcome = loop {
            let status = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
            if status == -1 {
                break Err(std::io::Error::last_os_error().to_string());
            }
            if status == 0 {
                break Ok(());
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        };
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        outcome
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = unsafe { &*(lp as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State };
    if !ptr.is_null() {
        let state = unsafe { &*ptr };
        if msg == WM_DEVICECHANGE {
            (state.invalidate)();
        }
        if msg == WM_TIMER && state.stop.load(Ordering::Acquire) {
            unsafe {
                DestroyWindow(hwnd);
            }
            return 0;
        }
    }
    if msg == WM_DESTROY {
        unsafe {
            KillTimer(hwnd, 1);
            PostQuitMessage(0);
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
