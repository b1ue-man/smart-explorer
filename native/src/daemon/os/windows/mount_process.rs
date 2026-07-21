use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

pub(super) const MOUNT_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
pub(super) const MOUNT_IPC_ADDR_ENV: &str = "SMART_EXPLORER_MOUNT_IPC_ADDR";
pub(super) const MOUNT_CACHE_DIR_ENV: &str = "SMART_EXPLORER_MOUNT_CACHE_DIR";

pub(super) fn spawn(
    mount_id: &crate::mount::MountId,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &std::path::Path,
) -> io::Result<Child> {
    use std::os::windows::process::CommandExt;

    let executable = mount_host_executable()?;
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
        .stderr(Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command.spawn()
}

fn mount_host_executable() -> io::Result<PathBuf> {
    let current = std::env::current_exe()?;
    if current
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("se.exe"))
    {
        return Ok(current);
    }
    let se = current.with_file_name("se.exe");
    if se.is_file() {
        Ok(se)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "se.exe fuer den isolierten Laufwerk-Host wurde nicht gefunden",
        ))
    }
}
