#[path = "os/shared/copy.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;

pub use imp::*;
