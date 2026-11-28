//! Deleting the file a DELETE handle holds, read-only ones included (U17,
//! 2026-11-27).
//!
//! `FileDispositionInfoEx` with IGNORE_READONLY_ATTRIBUTE deletes a
//! read-only file at once where the filesystem offers it (NTFS, ReFS). FAT,
//! FAT32 and exFAT, older systems and some providers answer it with "not
//! supported"; their plain `FileDispositionInfo` refuses a read-only file
//! (ERROR_ACCESS_DENIED). There the attribute is lifted on the same handle,
//! the file is marked for deletion and, should that fail, the attribute is
//! set again: the file the handle holds is the only one touched, as
//! `delete_own` in `transfer/src/copy/os/windows_fast.rs` does for its stage.
use std::{fs::File, io, mem::size_of, os::windows::io::AsRawHandle};
use windows_sys::Win32::Storage::FileSystem::{
    FileBasicInfo, FileDispositionInfo, FileDispositionInfoEx, GetFileInformationByHandleEx,
    SetFileInformationByHandle, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO,
    FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX,
};

/// FILE_DISPOSITION_FLAG_DELETE | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE.
const DELETE_POSIX_IGNORE_READONLY: u32 = 0x1 | 0x2 | 0x10;

/// Deletes the file `guard` holds. `guard` has DELETE access, and
/// FILE_WRITE_ATTRIBUTES as well when `readonly` (the file is read-only).
pub fn delete_held(guard: &File, readonly: bool) -> io::Result<()> {
    let mut disposition = FILE_DISPOSITION_INFO_EX {
        Flags: DELETE_POSIX_IGNORE_READONLY,
    };
    // SAFETY: a live handle and a complete FILE_DISPOSITION_INFO_EX.
    if unsafe {
        SetFileInformationByHandle(
            guard.as_raw_handle(),
            FileDispositionInfoEx,
            (&mut disposition as *mut FILE_DISPOSITION_INFO_EX).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } != 0
    {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    // A provider that takes the call but not the IGNORE_READONLY flag
    // refuses the read-only file as denied: the plain way below as well.
    let denied_read_only = readonly && error.raw_os_error() == Some(5);
    if !disposition_ex_unsupported(&error) && !denied_read_only {
        return Err(error);
    }
    if !readonly {
        return mark_deleted(guard);
    }
    let attributes = basic_info(guard)?.FileAttributes;
    set_attributes(guard, writable_attributes(attributes))?;
    match mark_deleted(guard) {
        Ok(()) => Ok(()),
        Err(error) => match set_attributes(guard, attributes) {
            Ok(()) => Err(error),
            Err(restore) => Err(io::Error::new(
                error.kind(),
                format!("{error}; the read-only attribute could not be set again: {restore}"),
            )),
        },
    }
}

/// The answers of a provider without `FileDispositionInfoEx`: invalid
/// function, not supported, invalid parameter, invalid level.
pub(super) fn disposition_ex_unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124))
}

/// `attributes` without READONLY. FILE_BASIC_INFO reads 0 as "leave the
/// attributes as they are", so a file left with none gets NORMAL.
pub(super) fn writable_attributes(attributes: u32) -> u32 {
    match attributes & !FILE_ATTRIBUTE_READONLY {
        0 => FILE_ATTRIBUTE_NORMAL,
        rest => rest,
    }
}

fn mark_deleted(guard: &File) -> io::Result<()> {
    let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
    // SAFETY: a live handle and a complete FILE_DISPOSITION_INFO.
    if unsafe {
        SetFileInformationByHandle(
            guard.as_raw_handle(),
            FileDispositionInfo,
            (&mut disposition as *mut FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn basic_info(file: &File) -> io::Result<FILE_BASIC_INFO> {
    // SAFETY: plain data the call fills completely on success.
    let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(basic)
}

/// Sets only the attributes: every time field 0 leaves that time as it is.
fn set_attributes(file: &File, attributes: u32) -> io::Result<()> {
    let basic = FILE_BASIC_INFO {
        CreationTime: 0,
        LastAccessTime: 0,
        LastWriteTime: 0,
        ChangeTime: 0,
        FileAttributes: attributes,
    };
    // SAFETY: a live handle and a complete FILE_BASIC_INFO.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileBasicInfo,
            (&basic as *const FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
#[path = "held_delete_tests.rs"]
mod tests;
