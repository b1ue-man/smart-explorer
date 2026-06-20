#[path = "core/entry.rs"]
mod core;
#[path = "os/shared.rs"]
mod os;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;

pub use os::*;
