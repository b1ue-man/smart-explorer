#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;

#[path = "core/backend.rs"]
mod backend;
#[path = "core/net.rs"]
mod imp;

pub use backend::UncBackend;
pub use imp::*;
