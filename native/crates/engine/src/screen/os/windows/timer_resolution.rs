//! The capture tick of a screen session on Windows (plan 2026-11-14
//! „Bildkette“ B-08, G9). The runtime's timer waits in whole system timer
//! periods, by default about 15.6 ms (Microsoft, „Timers, Timer Resolution,
//! and Development of Efficient Code“), so a 60, 120 or 240 Hz tick lands up
//! to a period late. For the lifetime of a session this timer therefore
//! - raises the system timer resolution of the process to the finest period
//!   the system reports (`timeGetDevCaps` → `wPeriodMin`, then
//!   `timeBeginPeriod`; ended with `timeEndPeriod`), as Sunshine does for its
//!   capture (plan R01 `src/platform/windows/misc.cpp`, `timeBeginPeriod`);
//! - asks Windows 11 not to ignore that request while the process shows no
//!   window (`SetProcessInformation` with `ProcessPowerThrottling`,
//!   `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION` in `ControlMask` and
//!   cleared in `StateMask`; the daemon has no visible window);
//! - waits on a high-resolution waitable timer
//!   (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`, Windows 10 1803 and later; a
//!   plain waitable timer otherwise) in a thread of its own, one absolute
//!   due time per frame interval, and wakes the session loop through a
//!   `Notify` permit: ticks missed while the loop worked collapse into one.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
    SetWaitableTimer, WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, TIMER_ALL_ACCESS,
};

/// `TIMECAPS` of `mmsystem.h`.
#[repr(C)]
struct TimeCaps {
    period_min: u32,
    period_max: u32,
}

/// `TIMERR_NOERROR` of `mmsystem.h`.
const TIMERR_NOERROR: u32 = 0;

// winmm is not among the enabled windows-sys features; the three functions
// are declared here as in `mmsystem.h`/`timeapi.h`.
#[link(name = "winmm")]
extern "system" {
    fn timeGetDevCaps(caps: *mut TimeCaps, size: u32) -> u32;
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

/// The finest system timer period, requested until drop.
struct TimerPeriod(u32);

impl TimerPeriod {
    fn begin() -> Option<Self> {
        let mut caps = TimeCaps {
            period_min: 0,
            period_max: 0,
        };
        // SAFETY: `caps` is a writable TIMECAPS of the size passed.
        let read = unsafe { timeGetDevCaps(&mut caps, std::mem::size_of::<TimeCaps>() as u32) };
        if read != TIMERR_NOERROR || caps.period_min == 0 {
            return None;
        }
        // SAFETY: plain value call; matched by `timeEndPeriod` in `drop`.
        (unsafe { timeBeginPeriod(caps.period_min) } == TIMERR_NOERROR)
            .then_some(Self(caps.period_min))
    }

    #[cfg(test)]
    fn period_ms(&self) -> u32 {
        self.0
    }
}

impl Drop for TimerPeriod {
    fn drop(&mut self) {
        // SAFETY: ends exactly the period `begin` requested.
        unsafe {
            timeEndPeriod(self.0);
        }
    }
}

/// Keeps the timer resolution request in effect for a process without a
/// visible window (Windows 11); earlier versions refuse the class and keep
/// honouring the request anyway.
fn honour_timer_resolution() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: the pseudo handle of this process and a structure of the size passed.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

/// A waitable timer, high-resolution where the system offers it.
fn waitable_timer() -> Option<(HANDLE, bool)> {
    for (flags, high) in [(CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, true), (0, false)] {
        // SAFETY: no attributes and no name; the handle is closed by the thread.
        let timer = unsafe {
            CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), flags, TIMER_ALL_ACCESS)
        };
        if !timer.is_null() {
            return Some((timer, high));
        }
    }
    None
}

/// Waits until `due` on `timer`; a timer that cannot be armed sleeps.
fn wait_until(timer: HANDLE, due: Instant) {
    let wait = due.saturating_duration_since(Instant::now());
    if wait.is_zero() {
        return;
    }
    // Negative due times are relative, in 100-ns units (SetWaitableTimer).
    let relative = -i64::try_from(wait.as_nanos() / 100).unwrap_or(i64::MAX);
    // SAFETY: `timer` is open for the whole thread; no completion routine.
    let armed = unsafe { SetWaitableTimer(timer, &relative, 0, None, std::ptr::null(), 0) };
    if armed == 0 {
        std::thread::sleep(wait);
        return;
    }
    // SAFETY: as above.
    unsafe {
        WaitForSingleObject(timer, INFINITE);
    }
}

/// The capture tick of one session; ends with drop.
pub struct SessionTimer {
    notify: Arc<Notify>,
    stop: Arc<AtomicBool>,
    high_resolution: bool,
    _period: Option<TimerPeriod>,
}

impl SessionTimer {
    /// Starts a tick every `interval`; `None` when no timer can be created
    /// (the caller then uses the runtime's interval).
    pub fn start(interval: Duration) -> Option<Self> {
        let interval = interval.max(Duration::from_micros(1));
        let period = TimerPeriod::begin();
        honour_timer_resolution();
        let (timer, high_resolution) = waitable_timer()?;
        let notify = Arc::new(Notify::new());
        let stop = Arc::new(AtomicBool::new(false));
        let raw = timer as usize;
        let (ticks, ended) = (notify.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("screen-session-clock".into())
            .spawn(move || {
                let timer = raw as HANDLE;
                let mut due = Instant::now() + interval;
                while !ended.load(Ordering::Acquire) {
                    wait_until(timer, due);
                    ticks.notify_one();
                    due += interval;
                    let now = Instant::now();
                    if due + interval < now {
                        due = now + interval;
                    }
                }
                // SAFETY: the thread owns the handle and closes it once.
                unsafe {
                    CloseHandle(timer);
                }
            });
        if spawned.is_err() {
            // SAFETY: the thread did not start; the handle is still ours.
            unsafe {
                CloseHandle(raw as HANDLE);
            }
            return None;
        }
        Some(Self {
            notify,
            stop,
            high_resolution,
            _period: period,
        })
    }

    /// Whether the waitable timer is the high-resolution kind.
    pub fn high_resolution(&self) -> bool {
        self.high_resolution
    }

    /// The next tick; a permit kept while nobody waited answers at once.
    pub async fn tick(&self) {
        self.notify.notified().await;
    }
}

impl Drop for SessionTimer {
    /// The thread ends after at most one more interval and closes its timer.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows: the session tick runs on a high-resolution waitable timer
    /// and the process holds the finest timer period while it runs.
    #[test]
    fn bildkette_task_rate_windows_session_timer_is_high_resolution() {
        let period = TimerPeriod::begin().expect("the system reports a timer period");
        assert!(period.period_ms() >= 1);
        drop(period);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let timer = SessionTimer::start(Duration::from_micros(4_166)).expect("timer");
        assert!(timer.high_resolution(), "Windows 10 1803+ offers it");
        assert!(timer._period.is_some());
        runtime.block_on(async {
            for _ in 0..24 {
                tokio::time::timeout(Duration::from_secs(1), timer.tick())
                    .await
                    .expect("a tick within one second");
            }
        });
    }
}
