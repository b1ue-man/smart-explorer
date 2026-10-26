pub(crate) mod host;
#[cfg(windows)]
#[path = "windows/mod.rs"]
pub(crate) mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os/mod.rs"]
pub(crate) mod platform;
#[cfg(target_os = "android")]
#[path = "android/mod.rs"]
pub(crate) mod platform;
#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
#[path = "shared/unsupported.rs"]
pub(crate) mod platform;
pub(crate) mod shared;
