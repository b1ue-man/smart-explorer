//! Public administrative status; no user access to the broker pipe is added.
use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
};
use windows_sys::Win32::{
    Foundation::{FILETIME, GENERIC_READ, WAIT_TIMEOUT},
    Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, READ_CONTROL,
    },
    System::{
        SystemInformation::GetTickCount64,
        Threading::{
            GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        },
    },
};
fn root() -> io::Result<PathBuf> {
    Ok(super::protected::paths()?
        .config
        .parent()
        .ok_or_else(|| io::Error::other("public broker status root is unavailable"))?
        .join("Smart Explorer Login Status"))
}
pub(crate) fn host_process_instance(pid: u32) -> io::Result<String> {
    let process = super::security::Handle::new(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    })?;
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit = created;
    let mut kernel = created;
    let mut user = created;
    if unsafe { GetProcessTimes(process.0, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { WaitForSingleObject(process.0, 0) } != WAIT_TIMEOUT {
        return Err(io::Error::other(
            "login activity process is no longer running or observable",
        ));
    }
    Ok(format!(
        "{}:{}",
        created.dwHighDateTime, created.dwLowDateTime
    ))
}
pub(crate) fn host_uptime_ms() -> io::Result<u64> {
    Ok(unsafe { GetTickCount64() })
}
pub(crate) fn read_host_evidence() -> io::Result<Vec<u8>> {
    let root = root()?;
    let _pin = super::protected::open_protected(&root.join("owner.marker"), false)?;
    // The immutable marker pins administrative ancestry. Only the changing
    // status leaf permits replacement; image/manifest sharing stays strict.
    let file = OpenOptions::new()
        .access_mode(GENERIC_READ | FILE_READ_ATTRIBUTES | READ_CONTROL)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root.join("status.json"))?;
    super::protected::validate_public_status(&file)?;
    if file.metadata()?.len() > 4096 {
        return Err(io::Error::other("public broker status exceeds limit"));
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(io::Error::other("public broker status exceeds limit"));
    }
    Ok(bytes)
}
pub(crate) fn write_host_evidence(bytes: &[u8]) -> io::Result<()> {
    super::security::require_system()?;
    if bytes.len() > 4096 {
        return Err(io::Error::other("public broker status exceeds limit"));
    }
    let root = root()?;
    let _pin = super::protected::open_protected(&root.join("owner.marker"), false)?;
    let temp = root.join(format!(".status-{}", crate::login::os::shared::nonce()?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, root.join("status.json"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
