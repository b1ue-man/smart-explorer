//! Threads of the screen host process on Windows (plan 2026-11-14
//! „Bildkette“ B-27, B-29).
//! - `raise_priority`: the capture and the encoding stage (and the input
//!   thread) run with `THREAD_PRIORITY_HIGHEST`, as Sunshine raises its
//!   capture thread (plan R01 `video.cpp`, priority `critical`).
//!   `SetThreadPriority` within the process's priority class needs no
//!   administrator right; the process class (`HIGH_PRIORITY_CLASS`) and the
//!   GPU scheduling priority stay untouched (plan 4.5 B-29: they act on the
//!   user's whole system).
//! - `PreciseWait`: the capture stage waits for its next tick on a
//!   high-resolution waitable timer (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`,
//!   Windows 10 1803 and later; a plain waitable timer otherwise), because
//!   `Sleep` waits in whole system timer periods of about 15.6 ms; the same
//!   timer the daemon's session tick uses (`timer_resolution.rs`, B-08).
use std::time::Instant;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentThread, SetThreadPriority, SetWaitableTimer,
    WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, THREAD_PRIORITY_HIGHEST,
    TIMER_ALL_ACCESS,
};

/// Raises the calling thread to `THREAD_PRIORITY_HIGHEST`; `false` when
/// Windows refused (the thread keeps its priority).
pub fn raise_priority() -> bool {
    // SAFETY: the pseudo handle of the calling thread; plain value call.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) != 0 }
}

/// Waits until an instant on a waitable timer of the calling thread.
pub struct PreciseWait {
    timer: HANDLE,
}

impl Default for PreciseWait {
    fn default() -> Self {
        for flags in [CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, 0] {
            // SAFETY: no attributes and no name; the handle is closed in drop.
            let timer = unsafe {
                CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), flags, TIMER_ALL_ACCESS)
            };
            if !timer.is_null() {
                return Self { timer };
            }
        }
        Self {
            timer: std::ptr::null_mut(),
        }
    }
}

impl PreciseWait {
    /// Returns at `due` (at once when it passed); without a timer it sleeps.
    pub fn until(&self, due: Instant) {
        let wait = due.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return;
        }
        if self.timer.is_null() {
            std::thread::sleep(wait);
            return;
        }
        // Negative due times are relative, in 100-ns units (SetWaitableTimer).
        let relative = -i64::try_from(wait.as_nanos() / 100).unwrap_or(i64::MAX);
        // SAFETY: `timer` is open until drop; no completion routine.
        let armed =
            unsafe { SetWaitableTimer(self.timer, &relative, 0, None, std::ptr::null(), 0) };
        if armed == 0 {
            std::thread::sleep(wait);
            return;
        }
        // SAFETY: as above.
        unsafe {
            WaitForSingleObject(self.timer, INFINITE);
        }
    }
}

impl Drop for PreciseWait {
    fn drop(&mut self) {
        if !self.timer.is_null() {
            // SAFETY: the handle was created by this value and is closed once.
            unsafe {
                CloseHandle(self.timer);
            }
        }
    }
}
