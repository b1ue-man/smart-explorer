#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;

pub(crate) const DAEMON_HANDOFF_ENV: &str = "SMART_EXPLORER_DAEMON_HANDOFF";
pub(crate) const DAEMON_RETIRING_GENERATION_ENV: &str = "SMART_EXPLORER_DAEMON_RETIRING_GENERATION";

pub use platform::*;
