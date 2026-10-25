#[cfg(windows)]
#[path = "windows/mod.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
mod platform;
#[cfg(target_os = "android")]
#[path = "android.rs"]
mod platform;
pub(super) use platform::*;
