#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;

#[path = "core/net.rs"]
mod imp;

pub use imp::*;
