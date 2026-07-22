use std::io;
use std::net::SocketAddr;
use std::process::{Command, Stdio};

use super::mount_host_process::MountHostProcess;

pub(super) const MOUNT_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
pub(super) const MOUNT_IPC_ADDR_ENV: &str = "SMART_EXPLORER_MOUNT_IPC_ADDR";
pub(super) const MOUNT_CACHE_DIR_ENV: &str = "SMART_EXPLORER_MOUNT_CACHE_DIR";

pub(super) fn spawn(
    mount_id: &crate::mount::MountId,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &std::path::Path,
) -> io::Result<MountHostProcess> {
    use std::os::windows::process::CommandExt;

    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .arg("--mount-host")
        .arg(mount_id.as_str())
        .env_clear()
        .env(MOUNT_TOKEN_ENV, launch_token)
        .env(MOUNT_IPC_ADDR_ENV, ipc_addr.to_string())
        .env(MOUNT_CACHE_DIR_ENV, cache_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    MountHostProcess::capture_piped_stderr(command.spawn()?)
}
