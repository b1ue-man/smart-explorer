use std::io;

use windows_sys::Win32::{
    Foundation::GetLastError,
    Storage::FileSystem::{GetDiskFreeSpaceExW, BY_HANDLE_FILE_INFORMATION, WIN32_FIND_DATAW},
};

use crate::mount::{DriveLetter, MountStatus};

use super::{
    callback_context::{context_key, NodeHandle},
    callback_status::{guard_with_context, insufficient_buffer, win32},
    dokany_abi::FillFindData,
    metadata::{file_information, find_data, reject_open_symlink},
    wide::{read_wide, write_wide},
    DokanFileInfo, NtStatus,
};

const FILE_CASE_SENSITIVE_SEARCH: u32 = 0x0000_0001;
const FILE_CASE_PRESERVED_NAMES: u32 = 0x0000_0002;
const FILE_UNICODE_ON_DISK: u32 = 0x0000_0004;
const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;

pub(super) unsafe extern "system" fn get_file_information(
    file_name: *const u16,
    output: *mut BY_HANDLE_FILE_INFORMATION,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_with_context(file_info, |context| {
            let path = read_wide(file_name)?;
            let meta = match context_key(file_info)
                .ok()
                .and_then(|key| context.snapshot(key).ok())
                .map(|snapshot| snapshot.node)
            {
                Some(NodeHandle::File(handle)) => context.engine.stat_handle(handle)?,
                _ => context.engine.stat_cached(&path)?,
            };
            let information =
                file_information(&meta, &path, context.volume_serial, context.read_only)?;
            let output = output.as_mut().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "missing file-information buffer",
                )
            })?;
            *output = information;
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn find_files(
    file_name: *const u16,
    fill: FillFindData,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_with_context(file_info, |context| {
            let path = read_wide(file_name)?;
            find_entries(context, &path, None, fill, file_info)
        })
    }
}

pub(super) unsafe extern "system" fn find_files_with_pattern(
    file_name: *const u16,
    search_pattern: *const u16,
    fill: FillFindData,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_with_context(file_info, |context| {
            let path = read_wide(file_name)?;
            let pattern = read_wide(search_pattern)?;
            find_entries(context, &path, Some(&pattern), fill, file_info)
        })
    }
}

pub(super) unsafe extern "system" fn get_disk_free_space(
    free_for_caller: *mut u64,
    total_bytes: *mut u64,
    total_free: *mut u64,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_with_context(file_info, |context| {
            if free_for_caller.is_null() || total_bytes.is_null() || total_free.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "missing disk-capacity output",
                )
                .into());
            }
            // The local whole-file spool is a hard lower capacity boundary.
            // Remote quotas are backend-specific and still surface on commit.
            if GetDiskFreeSpaceExW(
                context.cache_root_wide.as_ptr(),
                free_for_caller,
                total_bytes,
                total_free,
            ) == 0
            {
                return Err(win32(GetLastError()));
            }
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn get_volume_information(
    volume_name: *mut u16,
    volume_name_size: u32,
    volume_serial_number: *mut u32,
    maximum_component_length: *mut u32,
    file_system_flags: *mut u32,
    file_system_name: *mut u16,
    file_system_name_size: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_with_context(file_info, |context| {
            let label = if context.label.is_empty() {
                "Smart Explorer"
            } else {
                &context.label
            };
            write_wide(volume_name, volume_name_size, label, Some(32))
                .map_err(|_| insufficient_buffer())?;
            write_wide(file_system_name, file_system_name_size, "SmartEx", None)
                .map_err(|_| insufficient_buffer())?;
            *volume_serial_number.as_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "missing volume serial output")
            })? = context.volume_serial;
            *maximum_component_length.as_mut().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "missing component-length output",
                )
            })? = 255;
            let flags = FILE_CASE_PRESERVED_NAMES
                | FILE_UNICODE_ON_DISK
                | if context.case_sensitive_paths {
                    FILE_CASE_SENSITIVE_SEARCH
                } else {
                    0
                }
                | if context.read_only {
                    FILE_READ_ONLY_VOLUME
                } else {
                    0
                };
            *file_system_flags.as_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "missing volume-flags output")
            })? = flags;
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn mounted(
    mount_point: *const u16,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_with_context(file_info, |context| {
            let mount_point = read_wide(mount_point)?;
            let mut characters = mount_point.chars();
            let letter = characters.next().ok_or_else(invalid_mount_point)?;
            if characters.next() != Some(':')
                || !matches!(
                    (characters.next(), characters.next()),
                    (None, None) | (Some('\\'), None)
                )
            {
                return Err(invalid_mount_point().into());
            }
            let drive = DriveLetter::parse(letter)?;
            context.set_selected_drive(drive)?;
            context.report(MountStatus::Mounted { drive });
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn unmounted(file_info: *mut DokanFileInfo) -> NtStatus {
    // Dokan invokes this before DokanCloseHandle has returned. The host owns the
    // final status so it can first verify that no retryable journal work remains.
    unsafe { guard_with_context(file_info, |_| Ok(())) }
}

fn invalid_mount_point() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Dokany returned a non-drive mount point",
    )
}

fn find_entries(
    context: &super::callback_context::CallbackContext,
    path: &str,
    pattern: Option<&str>,
    fill: FillFindData,
    file_info: *mut DokanFileInfo,
) -> super::callback_status::CallbackResult {
    // Loading a missing directory snapshot also captures its own metadata;
    // ask for that first so a cold FindFiles callback does not issue a
    // separate point-stat immediately before the same directory fetch.
    let entries = context.engine.list_dir_cached(path)?;
    let directory = context.engine.stat_cached(path)?;
    reject_open_symlink(&directory)?;
    if !directory.is_dir {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is not a directory").into());
    }
    if !is_mount_root(path) {
        for special in [".", ".."] {
            if matches_pattern(context, pattern, special) {
                let mut data = find_data(&directory, special, context.read_only, true)?;
                if fill_entry(fill, &mut data, file_info) == FillDisposition::Full {
                    return Ok(());
                }
            }
        }
    }
    for entry in entries.iter() {
        // Dokany would treat FILE_ATTRIBUTE_REPARSE_POINT as usable only when
        // GetReparsePoint is implemented. Hiding a link is safer than exposing
        // a path Windows could mistake for a traversable reparse point.
        if entry.is_symlink {
            continue;
        }
        if matches_pattern(context, pattern, &entry.name) {
            let mut data = find_data(entry, &entry.name, context.read_only, false)?;
            if fill_entry(fill, &mut data, file_info) == FillDisposition::Full {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn matches_pattern(
    context: &super::callback_context::CallbackContext,
    pattern: Option<&str>,
    name: &str,
) -> bool {
    let Some(pattern) = pattern else {
        return true;
    };
    let pattern = nul_terminated(pattern);
    let name = nul_terminated(name);
    unsafe {
        context.runtime.is_name_in_expression(
            pattern.as_ptr(),
            name.as_ptr(),
            !context.case_sensitive_paths,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FillDisposition {
    Continue,
    Full,
}

fn fill_entry(
    fill: FillFindData,
    data: &mut WIN32_FIND_DATAW,
    file_info: *mut DokanFileInfo,
) -> FillDisposition {
    fill_disposition(unsafe { fill(data, file_info) })
}

fn fill_disposition(result: i32) -> FillDisposition {
    // FillFindData reports a full caller buffer with a nonzero return. Dokany
    // expects enumeration to stop successfully in that case.
    if result == 0 {
        FillDisposition::Continue
    } else {
        FillDisposition::Full
    }
}

fn is_mount_root(path: &str) -> bool {
    path.trim_matches(|character| matches!(character, '\\' | '/'))
        .is_empty()
}

fn nul_terminated(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_dokany_root_omits_dot_entries() {
        assert!(is_mount_root("\\"));
        assert!(is_mount_root("/"));
        assert!(!is_mount_root("\\folder"));
    }

    #[test]
    fn remote_drive_task_dokany_full_find_buffer_stops_successfully() {
        assert_eq!(fill_disposition(0), FillDisposition::Continue);
        assert_eq!(fill_disposition(1), FillDisposition::Full);
    }
}
