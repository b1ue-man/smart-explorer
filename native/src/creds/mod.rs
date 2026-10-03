#[path = "core/types.rs"]
mod core;
#[path = "os/shared.rs"]
mod os;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod secure_store;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod secure_store;
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
