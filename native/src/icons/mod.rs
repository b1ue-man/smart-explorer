#[path = "core/icons.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "os/shared.rs"]
mod os;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod os;

pub use imp::*;
