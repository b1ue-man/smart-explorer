use super::super::core::lifecycle::{ProcessIdentity, StopIdentity};
use std::fs::{File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, LOCKFILE_EXCLUSIVE_LOCK,
    LOCKFILE_FAIL_IMMEDIATELY,
};
#[path = "environment.rs"]
mod environment;
#[path = "persistence.rs"]
mod persistence;
#[path = "security.rs"]
mod security;
#[path = "spawn.rs"]
mod spawn;
pub(in crate::mcp_launcher) use persistence::*;
pub(in crate::mcp_launcher) use spawn::*;
pub(in crate::mcp_launcher) const CLI_NAME: &str = "se.exe";
pub(in crate::mcp_launcher) const LAUNCHER_NAME: &str = "se-mcp-launcher.exe";

pub(in crate::mcp_launcher) fn validate_tree(root: &Path, path: &Path) -> Result<(), String> {
    if !root.is_absolute() || !path.starts_with(root) {
        return Err("runtime_path_outside_installation".into());
    }
    if root
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .starts_with("\\\\")
    {
        return Err("runtime_network_installation_unsupported".into());
    }
    use std::os::windows::ffi::OsStrExt;
    let root_wide = root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut volume = vec![0u16; 32768];
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW(
            root_wide.as_ptr(),
            volume.as_mut_ptr(),
            volume.len() as u32,
        )
    } == 0
    {
        return Err("runtime_volume_check_failed".into());
    }
    let drive = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(volume.as_ptr()) };
    if matches!(drive, 0 | 1 | 4) {
        return Err("runtime_network_installation_unsupported".into());
    }
    for ancestor in root.ancestors().skip(1) {
        security::check_ancestor(ancestor)?;
    }
    let owner = security::owner(root)?;
    let mut current = root.to_path_buf();
    security::check(&current, &owner)?;
    for component in path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .components()
    {
        let std::path::Component::Normal(name) = component else {
            return Err("runtime_path_component_invalid".into());
        };
        current.push(name);
        security::check(&current, &owner)?;
    }
    Ok(())
}
pub(in crate::mcp_launcher) fn open_verified(
    root: &Path,
    path: &Path,
    hash: &str,
) -> Result<File, String> {
    validate_tree(root, path)?;
    // No FILE_SHARE_WRITE/DELETE: verified bytes and namespace stay pinned through CreateProcessW.
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err("runtime_reparse_or_nonfile".into());
    }
    let mut final_path = vec![0u16; 32768];
    let length = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW(
            file.as_raw_handle() as _,
            final_path.as_mut_ptr(),
            final_path.len() as u32,
            0,
        )
    };
    if length == 0 || length as usize >= final_path.len() {
        return Err("runtime_final_image_path_unavailable".into());
    }
    let final_path = String::from_utf16_lossy(&final_path[..length as usize]);
    let expected = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    if !final_path.eq_ignore_ascii_case(&expected.to_string_lossy()) {
        return Err("runtime_final_image_path_mismatch".into());
    }
    super::super::installation::verify_handle(&mut file, hash)?;
    Ok(file)
}
pub(in crate::mcp_launcher) fn lock(path: &Path, exclusive: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(7)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| format!("lock {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("lock_not_regular".into());
    }
    let flags = LOCKFILE_FAIL_IMMEDIATELY
        | if exclusive {
            LOCKFILE_EXCLUSIVE_LOCK
        } else {
            0
        };
    let mut overlapped = unsafe { std::mem::zeroed() };
    if unsafe { LockFileEx(file.as_raw_handle() as _, flags, 0, 1, 0, &mut overlapped) } == 0 {
        let error = std::io::Error::last_os_error();
        return Err(format!(
            "{}: {}: {error}",
            if error.raw_os_error() == Some(33) {
                "lock_busy"
            } else {
                "lock_denied"
            },
            path.display()
        ));
    }
    Ok(file)
}
pub(in crate::mcp_launcher) fn legacy_profile_image(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err("integration_legacy_image_not_regular".into());
    }
    Ok(file)
}
pub(in crate::mcp_launcher) fn legacy_update_image(_path: &Path) -> Result<Option<File>, String> {
    // The Windows terminal-only updater refuses replacement before UpdateLock.
    // Its test-only swap helper retains the existing namespace-lock behavior.
    Ok(None)
}
pub(in crate::mcp_launcher) fn same_image_file(file: &File, path: &Path) -> Result<bool, String> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let identity = |file: &File| -> Result<(u32, u32, u32), String> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    };
    let named = legacy_profile_image(path)?;
    Ok(identity(file)? == identity(&named)?)
}
pub(in crate::mcp_launcher) fn protect(path: &Path, _executable: bool) -> Result<(), String> {
    security::protect(path)
}
pub(in crate::mcp_launcher) fn process_identity(pid: u32) -> Result<ProcessIdentity, String> {
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let raw = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if raw.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let handle = std::os::windows::io::OwnedHandle::from_raw_handle(raw as _);
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        if QueryFullProcessImageNameW(raw, 0, path.as_mut_ptr(), &mut length) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        if GetProcessTimes(raw, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        drop(handle);
        Ok(ProcessIdentity {
            executable_object: None,
            pid,
            started: format!("{}:{}", created.dwHighDateTime, created.dwLowDateTime),
            image: String::from_utf16_lossy(&path[..length as usize]),
        })
    }
}
use std::os::windows::io::FromRawHandle;
pub(in crate::mcp_launcher) fn processes_for(path: &Path) -> Result<Vec<ProcessIdentity>, String> {
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let _owner = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(snapshot as _) };
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut next = unsafe { Process32FirstW(snapshot, &mut entry) };
    let mut result = Vec::new();
    let target = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace("\\\\?\\", "");
    let wanted = path
        .file_name()
        .ok_or("runtime_image_name_missing")?
        .to_string_lossy();
    while next != 0 {
        let end = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if entry.th32ProcessID > 4 && name.eq_ignore_ascii_case(&wanted) {
            match process_identity(entry.th32ProcessID) {
                Ok(identity)
                    if identity.image.to_ascii_lowercase().replace("\\\\?\\", "") == target =>
                {
                    result.push(identity)
                }
                Ok(_) => {}
                Err(_) => return Err("runtime_process_status_unknown".into()),
            }
        }
        next = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    Ok(result)
}
pub(in crate::mcp_launcher) fn stop_identity(path: &Path) -> Result<StopIdentity, String> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(StopIdentity {
        file_id: format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ),
        modified: format!(
            "{}:{}:{}:{}",
            info.ftLastWriteTime.dwHighDateTime,
            info.ftLastWriteTime.dwLowDateTime,
            info.nFileSizeHigh,
            info.nFileSizeLow
        ),
        sha256: super::super::installation::hash_bytes(
            &std::fs::read(path).map_err(|e| e.to_string())?,
        ),
    })
}

pub(in crate::mcp_launcher) fn process_matches(expected: &ProcessIdentity) -> bool {
    process_identity(expected.pid).is_ok_and(|actual| actual == *expected)
}

/// Uninstall never terminates a process by its basename. Keep every active
/// owner alive and let the user close this installation before retrying.
pub(in crate::mcp_launcher) fn uninstall_programs_idle(root: &Path) -> Result<(), String> {
    for name in [
        CLI_NAME,
        "Smart Explorer.exe",
        "smart_explorer.exe",
        "Smart Explorer Updater.exe",
        "smart_explorer_updater.exe",
    ] {
        let owners = processes_for(&root.join(name))?;
        if owners.iter().any(|owner| owner.pid != std::process::id()) {
            return Err(format!("uninstall_busy: {} has active program owners; close Smart Explorer, its background worker and this installation's terminal commands, then retry",root.join(name).display()));
        }
    }
    Ok(())
}
