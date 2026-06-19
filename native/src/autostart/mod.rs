#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "os/non_windows.rs"]
mod platform;

pub use platform::*;
