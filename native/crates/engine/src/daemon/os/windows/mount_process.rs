use std::ffi::OsString;
use std::io;
use std::net::SocketAddr;
use std::os::windows::ffi::OsStringExt;

use super::mount_host_process::MountHostProcess;
use super::{mount_launch, mount_process_environment};
use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;

pub(super) use mount_process_environment::{
    MOUNT_CACHE_DIR_ENV, MOUNT_IPC_ADDR_ENV, MOUNT_TOKEN_ENV,
};

const INITIAL_WINDOWS_DIRECTORY_CAPACITY: usize = 260;
const MAX_WINDOWS_DIRECTORY_UNITS: usize = 32_768;

pub(super) fn spawn(
    mount_id: &crate::mount::MountId,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &std::path::Path,
) -> io::Result<MountHostProcess> {
    let system_windows_directory = system_windows_directory()?;
    let launched = mount_launch::launch(
        mount_id,
        &system_windows_directory,
        launch_token,
        ipc_addr,
        cache_root,
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Laufwerk-Host abgesichert starten: {error}"),
        )
    })?;
    MountHostProcess::capture_piped_stderr(launched.child, launched.stderr, launched.job)
}

fn system_windows_directory() -> io::Result<OsString> {
    let mut buffer = vec![0u16; INITIAL_WINDOWS_DIRECTORY_CAPACITY];
    loop {
        let length = unsafe { GetSystemWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) }
            as usize;
        if length == 0 {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(
                error.kind(),
                format!("Windows-Systemverzeichnis fuer Laufwerk-Host bestimmen: {error}"),
            ));
        }
        if length < buffer.len() {
            buffer.truncate(length);
            return Ok(OsString::from_wide(&buffer));
        }
        let capacity = length.checked_add(1).ok_or_else(directory_too_long)?;
        if capacity > MAX_WINDOWS_DIRECTORY_UNITS {
            return Err(directory_too_long());
        }
        buffer.resize(capacity, 0);
    }
}

fn directory_too_long() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Windows-Systemverzeichnis fuer Laufwerk-Host ist zu lang",
    )
}
