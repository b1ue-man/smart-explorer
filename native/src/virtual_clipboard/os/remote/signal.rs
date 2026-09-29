//! Kernel events that COM methods can wait on without stalling their
//! apartment: on an STA the wait keeps dispatching incoming calls and sent
//! messages (for example a clipboard owner change), so a slow listing or
//! network read never blocks another thread that talks to this one.
use std::time::Duration;
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, RPC_S_CALLPENDING, WAIT_OBJECT_0};
use windows::Win32::System::Com::{CoWaitForMultipleHandles, COWAIT_DEFAULT};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

/// Back-stop for a missed wake-up and the interval of cancellation checks
/// while waiting; a notification itself wakes the waiter at once.
pub(super) const WAIT_SLICE: Duration = Duration::from_millis(100);

pub(super) struct Signal(HANDLE);

// SAFETY: an event handle is a process-wide kernel object. SetEvent and the
// waits are thread-safe, and the handle is closed exactly once, in Drop.
unsafe impl Send for Signal {}
unsafe impl Sync for Signal {}

impl Signal {
    /// A `sticky` signal stays set once notified (one-time completion);
    /// otherwise each notification wakes one waiter, and a notification that
    /// arrives before the wait is kept until then.
    pub(super) fn new(sticky: bool) -> Result<Self> {
        let handle = unsafe { CreateEventW(None, sticky, false, PCWSTR::null())? };
        Ok(Self(handle))
    }

    pub(super) fn notify(&self) {
        // SetEvent on a valid event handle has no failure a caller could act on.
        let _ = unsafe { SetEvent(self.0) };
    }

    /// Waits at most `timeout`; true when notified.
    pub(super) fn wait(&self, timeout: Duration) -> bool {
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        match unsafe { CoWaitForMultipleHandles(COWAIT_DEFAULT.0 as u32, millis, &[self.0]) } {
            Ok(_) => true,
            Err(error) if error.code() == RPC_S_CALLPENDING => false,
            // Only direct calls from a thread without COM get here.
            Err(_) => unsafe { WaitForSingleObject(self.0, millis) == WAIT_OBJECT_0 },
        }
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
