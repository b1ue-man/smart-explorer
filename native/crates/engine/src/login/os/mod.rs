//! The only platform selection boundary of the login component.
#[cfg(windows)]
#[path = "windows/mod.rs"]
pub mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
pub mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "unavailable.rs"]
pub mod platform;
pub mod shared;
