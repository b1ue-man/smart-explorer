pub mod host;
#[cfg(windows)]
#[path = "windows/mod.rs"]
pub mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
pub mod platform;
#[cfg(target_os = "android")]
#[path = "android/mod.rs"]
pub mod platform;
#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
#[path = "shared/unsupported.rs"]
pub mod platform;
pub mod shared;
