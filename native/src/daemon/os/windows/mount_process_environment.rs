use std::{
    ffi::{OsStr, OsString},
    net::SocketAddr,
    path::Path,
};

#[cfg(test)]
use crate::mount::MountId;
#[cfg(test)]
use std::process::Command;

pub(super) const MOUNT_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
pub(super) const MOUNT_IPC_ADDR_ENV: &str = "SMART_EXPLORER_MOUNT_IPC_ADDR";
pub(super) const MOUNT_CACHE_DIR_ENV: &str = "SMART_EXPLORER_MOUNT_CACHE_DIR";

#[cfg(test)]
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
        .envs(values(
            system_windows_directory,
            launch_token,
            ipc_addr,
            cache_root,
        ));
}

pub(super) fn values(
    system_windows_directory: &OsStr,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &Path,
) -> Vec<(&'static str, OsString)> {
    vec![
        ("SystemRoot", system_windows_directory.to_os_string()),
        ("WINDIR", system_windows_directory.to_os_string()),
        (MOUNT_TOKEN_ENV, OsString::from(launch_token)),
        (MOUNT_IPC_ADDR_ENV, OsString::from(ipc_addr.to_string())),
        (MOUNT_CACHE_DIR_ENV, cache_root.as_os_str().to_os_string()),
    ]
}
