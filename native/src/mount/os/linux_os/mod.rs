//! Drives under Linux and Android.
//!
//! The cache lease and the free-space probe (DRV-M2) serve the FUSE host
//! and the Android documents provider (DRV-M8). The FUSE host (DRV-M4) and
//! everything it needs exist only for Linux: `fuser` is a Linux dependency
//! (`fuser/build.rs` has no Android branch without pkg-config), and its
//! types stay inside `fuse_*.rs`.
// Android uses the cache lease and the space probe from DRV-M8 on.
#![cfg_attr(target_os = "android", allow(dead_code))]

pub(crate) mod cache_lease;
pub(crate) mod cache_space;

// DRV-M5: the places of the file managers and „Im Dateimanager öffnen“.
#[cfg(target_os = "linux")]
pub(crate) mod bookmarks;
#[cfg(target_os = "linux")]
mod bookmarks_gtk;
#[cfg(target_os = "linux")]
mod bookmarks_kde;
#[cfg(all(test, target_os = "linux"))]
mod bookmarks_tests;
#[cfg(target_os = "linux")]
mod file_manager_open;
#[cfg(target_os = "linux")]
pub(crate) mod fuse_abort;
#[cfg(target_os = "linux")]
mod fuse_errno;
#[cfg(target_os = "linux")]
mod fuse_fs;
#[cfg(target_os = "linux")]
pub(crate) mod fuse_host;
#[cfg(target_os = "linux")]
mod fuse_inodes;
#[cfg(target_os = "linux")]
mod fuse_notify;
#[cfg(target_os = "linux")]
mod fuse_ops_attr;
#[cfg(target_os = "linux")]
mod fuse_ops_io;
#[cfg(target_os = "linux")]
mod fuse_ops_meta;
#[cfg(target_os = "linux")]
mod fuse_ops_ns;
#[cfg(target_os = "linux")]
pub(crate) mod fuse_runtime;
#[cfg(target_os = "linux")]
mod fuse_workers;
#[cfg(target_os = "linux")]
pub(crate) mod mount_point;
// VLT-M12: end-to-end signal of vault drives over FUSE.
#[cfg(all(test, target_os = "linux"))]
mod vault_fuse_task_tests;

pub(crate) use cache_lease::{audit_pending, audit_pending_state, audit_recovery, CacheLease};
#[cfg(target_os = "linux")]
pub(crate) use file_manager_open::{show_folders, OpenMethod};
#[cfg(target_os = "linux")]
pub(crate) use fuse_host::run_mount_host;
#[cfg(target_os = "linux")]
pub use fuse_runtime::{FuseInstallOutcome, FuseRuntimeReport};

/// `se drive runtime` under Linux; `version` runs `fusermount3 -V`.
#[cfg(target_os = "linux")]
pub fn fuse_runtime_report(version: bool) -> FuseRuntimeReport {
    fuse_runtime::report(&fuse_runtime::RuntimeProbe::system(version))
}

/// `se drive install-runtime` under Linux: se installs no system package.
#[cfg(target_os = "linux")]
pub fn fuse_install_outcome() -> FuseInstallOutcome {
    fuse_runtime::install_outcome(&fuse_runtime::RuntimeProbe::system(false))
}

/// Ends the se drive mounted at `folder` from outside its host, as the
/// file manager's eject does: without `force` a busy drive stays mounted
/// and the error is `ResourceBusy`; with `force` it is unmounted lazily and
/// its connection aborted, so programs blocked in it return `ENOTCONN`.
/// Nothing mounted there is success.
#[cfg(target_os = "linux")]
pub fn unmount_drive_folder(folder: &std::path::Path, force: bool) -> std::io::Result<()> {
    let canonical = mount_point::canonical(folder)?;
    let Some(entry) = fuse_abort::mount_at(&canonical) else {
        return Ok(());
    };
    if entry.fstype != mount_point::FUSE_TYPE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} ist kein Smart-Explorer-Laufwerk", folder.display()),
        ));
    }
    if !force {
        return mount_point::unmount(&canonical, false);
    }
    // The name first, then the connection: the host then sees an outside
    // unmount, not an abort of a still mounted drive.
    let unmounted = mount_point::unmount(&canonical, true);
    let aborted = fuse_abort::abort_connection(fuse_abort::connection_number(&entry));
    match (unmounted, aborted) {
        (Ok(()), _) => Ok(()),
        (Err(_), Ok(())) => {
            // An aborted connection no longer keeps the name busy.
            let _ = mount_point::unmount(&canonical, true);
            Ok(())
        }
        (Err(error), Err(_)) => Err(error),
    }
}
