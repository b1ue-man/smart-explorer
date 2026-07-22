use std::{ffi::OsStr, net::SocketAddr, path::Path, process::Command};

use crate::mount::MountId;

pub(super) const MOUNT_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
pub(super) const MOUNT_IPC_ADDR_ENV: &str = "SMART_EXPLORER_MOUNT_IPC_ADDR";
pub(super) const MOUNT_CACHE_DIR_ENV: &str = "SMART_EXPLORER_MOUNT_CACHE_DIR";

pub(super) fn configure(
    command: &mut Command,
    mount_id: &MountId,
    system_windows_directory: &OsStr,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &Path,
) {
    // Winsock provider paths may contain either system-directory alias. Keep
    // the child isolated from parent secrets while permitting that expansion.
    command
        .arg("--mount-host")
        .arg(mount_id.as_str())
        .env_clear()
        .env("SystemRoot", system_windows_directory)
        .env("WINDIR", system_windows_directory)
        .env(MOUNT_TOKEN_ENV, launch_token)
        .env(MOUNT_IPC_ADDR_ENV, ipc_addr.to_string())
        .env(MOUNT_CACHE_DIR_ENV, cache_root);
}
