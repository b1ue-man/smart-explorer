//! Windows lock signals of the worker (C3.4):
//!
//! - Sleep: `PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK)`,
//!   the event `PBT_APMSUSPEND` locks the sites that lock on sleep; the
//!   hidden window below also hears `WM_POWERBROADCAST` as a second way.
//! - Screen lock: a hidden top-level window (`WS_POPUP`, never shown; not
//!   `HWND_MESSAGE`, whose delivery is unverified) on a thread of its own
//!   with a message loop, registered with
//!   `WTSRegisterSessionNotification(NOTIFY_FOR_THIS_SESSION)`;
//!   `WM_WTSSESSION_CHANGE`/`WTS_SESSION_LOCK` locks. When the Remote Desktop
//!   Services are not ready yet (`RPC_S_INVALID_BINDING`), registration waits
//!   for `Global\TermSrvReadyEvent` and tries once more. Unverified
//!   assumption A1: Windows delivers the message to this never-shown window;
//!   otherwise the screen lock does not lock, sleep, idle and explicit locks
//!   still do.
//!
//! The window is a `STATIC` window whose procedure is replaced
//! (`GWLP_WNDPROC`), so no window class needs `Win32_Graphics_Gdi`.
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Weak};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_SUCCESS, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    PowerRegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    OpenEventW, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, TranslateMessage,
    DEVICE_NOTIFY_CALLBACK, GWLP_USERDATA, GWLP_WNDPROC, MSG, PBT_APMSUSPEND, WM_POWERBROADCAST,
    WM_WTSSESSION_CHANGE, WS_POPUP, WTS_SESSION_LOCK,
};

use crate::vault::core::lock_policy::{SignalControl, SignalSink, SignalSupport, SystemSignal};

/// Windows needs no note next to its screen lock switch.
pub(super) const SCREEN_LOCK_NOTE: Option<&str> = None;
/// `RPC_S_INVALID_BINDING` (`Win32_System_Rpc` is no feature of this crate).
const RPC_S_INVALID_BINDING: u32 = 1702;
/// How long the registration waits for the Remote Desktop Services.
const TERMINAL_SERVICES_WAIT: Duration = Duration::from_secs(600);

/// Shared with the window procedure and the power callback for the life of
/// the process (both hold its address).
struct Context {
    sink: Weak<dyn SignalSink>,
    sleep: AtomicBool,
    screen_lock: AtomicBool,
}

impl Context {
    fn signal(&self, signal: SystemSignal) {
        if let Some(sink) = self.sink.upgrade() {
            sink.signal(signal);
        }
    }
}

/// The running adapter.
struct WindowsSignals {
    context: &'static Context,
    /// The hidden window (`HWND` as integer; 0 = none).
    window: isize,
}

impl SignalControl for WindowsSignals {
    fn set_armed(&self, _armed: bool) {}

    fn support(&self) -> SignalSupport {
        SignalSupport {
            sleep: self.context.sleep.load(Ordering::SeqCst),
            screen_lock: self.context.screen_lock.load(Ordering::SeqCst),
            background: false,
        }
    }
}

pub(super) fn start_signal_watch(sink: Weak<dyn SignalSink>) -> Box<dyn SignalControl> {
    Box::new(start(sink))
}

fn start(sink: Weak<dyn SignalSink>) -> WindowsSignals {
    let context: &'static Context = Box::leak(Box::new(Context {
        sink,
        sleep: AtomicBool::new(false),
        screen_lock: AtomicBool::new(false),
    }));
    let power = register_power(context);
    let window = spawn_window(context).unwrap_or(0);
    // A top-level window hears `WM_POWERBROADCAST` as well.
    context.sleep.store(power || window != 0, Ordering::SeqCst);
    WindowsSignals { context, window }
}

unsafe extern "system" fn power_callback(
    context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    if kind == PBT_APMSUSPEND {
        if let Some(context) = (context as *const Context).as_ref() {
            context.signal(SystemSignal::Sleep);
        }
    }
    ERROR_SUCCESS
}

fn register_power(context: &'static Context) -> bool {
    let parameters: &'static DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
        Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_callback),
            Context: context as *const Context as *mut c_void,
        }));
    let mut registration: *mut c_void = null_mut();
    let status = unsafe {
        PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            parameters as *const DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as HANDLE,
            &mut registration,
        )
    };
    status == ERROR_SUCCESS
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_pointer_width = "64")]
unsafe fn set_window_long(window: HWND, index: i32, value: isize) {
    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(window, index, value);
}

#[cfg(target_pointer_width = "64")]
unsafe fn get_window_long(window: HWND, index: i32) -> isize {
    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(window, index)
}

#[cfg(target_pointer_width = "32")]
unsafe fn set_window_long(window: HWND, index: i32, value: isize) {
    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongW(window, index, value as i32);
}

#[cfg(target_pointer_width = "32")]
unsafe fn get_window_long(window: HWND, index: i32) -> isize {
    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongW(window, index) as isize
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let signal = match message {
        WM_WTSSESSION_CHANGE if wparam as u32 == WTS_SESSION_LOCK => {
            Some(SystemSignal::ScreenLocked)
        }
        WM_POWERBROADCAST if wparam as u32 == PBT_APMSUSPEND => Some(SystemSignal::Sleep),
        _ => None,
    };
    if let Some(signal) = signal {
        if let Some(context) = (get_window_long(window, GWLP_USERDATA) as *const Context).as_ref() {
            context.signal(signal);
        }
    }
    DefWindowProcW(window, message, wparam, lparam)
}

/// The hidden window; 0 when Windows refused it.
unsafe fn create_window(context: &'static Context) -> isize {
    let class = wide("STATIC");
    let title = wide("Smart Explorer Tresore");
    let window = CreateWindowExW(
        0,
        class.as_ptr(),
        title.as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    if window.is_null() {
        return 0;
    }
    set_window_long(window, GWLP_USERDATA, context as *const Context as isize);
    set_window_long(window, GWLP_WNDPROC, window_proc as usize as isize);
    window as isize
}

/// `Err` carries `GetLastError` of the failed registration.
fn register_sessions(window: isize) -> Result<(), u32> {
    if unsafe { WTSRegisterSessionNotification(window as HWND, NOTIFY_FOR_THIS_SESSION) } != 0 {
        Ok(())
    } else {
        Err(unsafe { GetLastError() })
    }
}

/// Waits until the Remote Desktop Services signal readiness.
fn wait_for_terminal_services() -> bool {
    let name = wide("Global\\TermSrvReadyEvent");
    let until = Instant::now() + TERMINAL_SERVICES_WAIT;
    loop {
        let event = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
        if !event.is_null() {
            let left = until.saturating_duration_since(Instant::now());
            let millis = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
            let waited = unsafe { WaitForSingleObject(event, millis) };
            unsafe { CloseHandle(event) };
            return waited == WAIT_OBJECT_0;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Creates the window on a thread of its own, registers it for session
/// notifications and runs its message loop; returns the window.
fn spawn_window(context: &'static Context) -> Option<isize> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("vault-session-window".into())
        .spawn(move || {
            let window = unsafe { create_window(context) };
            let _ = sender.send(window);
            if window == 0 {
                return;
            }
            match register_sessions(window) {
                Ok(()) => context.screen_lock.store(true, Ordering::SeqCst),
                Err(RPC_S_INVALID_BINDING) => {
                    let _ = std::thread::Builder::new()
                        .name("vault-session-ready".into())
                        .spawn(move || {
                            if wait_for_terminal_services() && register_sessions(window).is_ok() {
                                context.screen_lock.store(true, Ordering::SeqCst);
                            }
                        });
                }
                Err(_) => {}
            }
            let mut message: MSG = unsafe { std::mem::zeroed() };
            while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        })
        .ok()?;
    receiver.recv().ok().filter(|window| *window != 0)
}

/// Available physical memory for the scrypt check of C1.3 (c), as
/// `transfer/os/windows.rs` reads it.
pub(super) fn available_memory() -> Option<u64> {
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullAvailPhys)
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::ptr::null;
    use std::sync::{Arc, Weak};

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, PBT_APMSUSPEND, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK,
        WTS_SESSION_UNLOCK,
    };

    use super::super::session_test_support::{
        is_unlocked, settings, test_register, unlock_again, unlocked_site,
    };
    use super::{power_callback, start, Context};
    use crate::vault::core::lock_policy::{LockPlatform, SignalSink};

    /// Sends `message` to the window and returns after its procedure ran.
    fn send(window: isize, message: u32, wparam: u32) {
        unsafe { SendMessageW(window as HWND, message, wparam as usize, 0) };
    }

    #[test]
    fn vault_task_windows_wts_window_dispatch() {
        let register = test_register(LockPlatform::Desktop);
        let on_screen = unlocked_site(&register, settings(None, false, true, None));
        let on_sleep = unlocked_site(&register, settings(None, true, false, None));
        let neither = unlocked_site(&register, settings(None, false, false, None));
        let weak = Arc::downgrade(&register);
        let sink: Weak<dyn SignalSink> = weak;
        let signals = start(sink);
        assert_ne!(signals.window, 0, "hidden top-level window created");
        // Unlocking the Windows session locks nothing.
        send(signals.window, WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK);
        for site in [&on_screen, &on_sleep, &neither] {
            assert!(is_unlocked(&register, &site.0));
        }
        // Locking it locks the sites with „Bei Bildschirmsperre sperren“.
        send(signals.window, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK);
        assert!(!is_unlocked(&register, &on_screen.0));
        assert!(on_screen.1.is_locked(), "key wiped");
        assert!(is_unlocked(&register, &on_sleep.0));
        // Sleep through the registered power callback …
        let context = signals.context as *const Context as *const c_void;
        assert_eq!(
            unsafe { power_callback(context, PBT_APMSUSPEND, null()) },
            0
        );
        assert!(!is_unlocked(&register, &on_sleep.0));
        // … and through `WM_POWERBROADCAST` of the window.
        let again = unlock_again(&register, &on_sleep.0, settings(None, true, false, None));
        send(signals.window, WM_POWERBROADCAST, PBT_APMSUSPEND);
        assert!(again.is_locked());
        assert!(is_unlocked(&register, &neither.0));
        // The delivery by Windows itself is not checked here (A1, S3 K13).
        assert!(signals
            .context
            .sleep
            .load(std::sync::atomic::Ordering::SeqCst));
    }
}
