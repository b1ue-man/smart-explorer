//! Recovery publication opens never change attributes, bytes or namespace.
use std::{ffi::{c_void, OsStr}, fs::File, io, os::windows::io::{AsRawHandle, FromRawHandle}};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, RtlNtStatusToDosError, STATUS_PENDING, WAIT_OBJECT_0},
    Storage::FileSystem::{ReOpenFile, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE},
    System::{IO::IO_STATUS_BLOCK, Threading::{WaitForSingleObject, INFINITE}},
};

#[link(name = "ntdll")]
extern "system" {
    fn NtFlushBuffersFileEx(file: HANDLE, flags: u32, parameters: *const c_void,
        parameter_bytes: u32, completion: *mut IO_STATUS_BLOCK) -> i32;
}

pub(super) fn open_file(parent: &File, name: &OsStr) -> io::Result<File> {
    // FILE_OPEN, NON_DIRECTORY_FILE, WRITE_THROUGH. Refusing write/delete
    // sharing prevents concurrent writers/renames during the publication proof.
    // In contrast to ordinary writes this never clears the read-only bit and
    // does not reject an already published hardlink merely for its link count.
    let file = super::native::open(parent, name, FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        1, 0x40 | 0x2, FILE_SHARE_READ)?;
    super::regular::validate_file(&file, true)?;
    Ok(file)
}

pub(super) fn open_move_file(parent: &File, name: &OsStr) -> io::Result<File> {
    // A move must retain this flush-capable handle across its own rename.
    // Readonly/access/sharing failure selects the established staged path.
    let file = super::native::open(parent, name, FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        1, 0x40 | 0x2, FILE_SHARE_READ | FILE_SHARE_DELETE)?;
    super::regular::validate_file(&file, true)?;
    Ok(file)
}

pub(super) fn confirm_namespace(parent: &File) -> io::Result<()> {
    // ReOpenFile retains the object, unlike reopening a diagnostic parent path.
    // WRITE access is necessary for a real flush; inability to acquire it is
    // deliberately an error rather than a write-through-rename assertion.
    let handle = unsafe { ReOpenFile(parent.as_raw_handle(), FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT) };
    if handle == INVALID_HANDLE_VALUE { return Err(io::Error::last_os_error()); }
    let file = unsafe { File::from_raw_handle(handle) };
    let class = super::super::directory::classify_open_file(&file)?;
    if !file.metadata()?.is_dir() || class.link_like || class.special {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "namespace flush requires the held physical directory"));
    }
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    // Normal flags (0) flush data, metadata and underlying storage cache.
    // No DATA_ONLY/NO_SYNC shortcut can confirm a published directory entry.
    let status = unsafe { NtFlushBuffersFileEx(file.as_raw_handle(), 0,
        std::ptr::null(), 0, completion.as_mut()) };
    let status = if status == STATUS_PENDING {
        if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            std::mem::forget((file, completion));
            return Err(io::Error::other("publication namespace flush completion unconfirmed"));
        }
        let status = unsafe { completion.Anonymous.Status };
        if status == STATUS_PENDING {
            std::mem::forget((file, completion));
            return Err(io::Error::other("publication namespace flush still pending"));
        }
        status
    } else { status };
    if status < 0 {
        Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32))
    } else if status != 0 {
        Err(io::Error::other("publication namespace flush did not confirm success"))
    } else { Ok(()) }
}
