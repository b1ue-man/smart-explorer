//! Host side of the mask lists: the sealed store in the app data, the side
//! records in the se secret store, the integrity guard and the guarded
//! paths. Platform facts (system configuration directories of the agent
//! hosts, case rules of paths) come from the per-OS adapter.
#[path = "shared/guard.rs"]
pub(crate) mod guard;
#[path = "shared/guarded.rs"]
pub(crate) mod guarded;
#[path = "shared/side.rs"]
mod side;
#[path = "shared/store.rs"]
pub(crate) mod store;
#[path = "shared/store_admin.rs"]
mod store_admin;

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "linux_os.rs"]
mod platform;

#[cfg(test)]
#[path = "shared/guarded_tests.rs"]
mod guarded_tests;
#[cfg(test)]
#[path = "shared/store_tests.rs"]
mod store_tests;
