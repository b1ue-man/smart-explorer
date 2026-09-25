#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(target_os = "android")]
#[path = "os/android.rs"]
mod platform;

pub(crate) const DAEMON_HANDOFF_ENV: &str = "SMART_EXPLORER_DAEMON_HANDOFF";
pub(crate) const DAEMON_RETIRING_GENERATION_ENV: &str = "SMART_EXPLORER_DAEMON_RETIRING_GENERATION";
/// True where the background worker is a thread of the app process (Android)
/// instead of a separately launched process that hands off to its successor.
pub(crate) const DAEMON_IN_PROCESS: bool = cfg!(target_os = "android");

pub use platform::*;
