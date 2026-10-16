#[path = "core/types.rs"]
mod core;
#[cfg(not(windows))]
#[path = "os/key_protector.rs"]
pub(crate) mod key_protector;
#[path = "os/shared.rs"]
mod os;
#[cfg_attr(windows, allow(dead_code))]
#[path = "core/sealed_record.rs"]
pub(crate) mod sealed_record;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod secure_store;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod secure_store;
#[cfg(target_os = "linux")]
#[path = "os/linux_systemd_creds.rs"]
mod systemd_creds;
#[path = "core/transaction.rs"]
mod transaction;

#[cfg(windows)]
#[path = "os/private_storage_windows.rs"]
pub(crate) mod private_storage;
#[cfg(not(windows))]
#[path = "os/private_storage_unix.rs"]
pub(crate) mod private_storage;

pub use core::*;
pub use os::*;
