//! Creating and replacing local files on Windows: names NTFS would turn into
//! alternate data streams are refused, and a read-only destination is
//! replaced (its attribute moves to the new file).
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path};

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, MoveFileExW, SetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_READONLY, INVALID_FILE_ATTRIBUTES, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH,
};

use crate::types::{win32_name_issue, Win32NameIssue};

fn wide(path: &Path) -> Vec<u16> {
    crate::local_access::normalize_scan_root(path)
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect()
}

/// Refuses to create a name Win32 rejects: `:` would write into an
/// alternate data stream of another file, `? * " < > |` and control
/// characters fail on NTFS anyway. Existing reserved names and names with a
/// trailing dot or space stay reachable through verbatim paths.
pub(crate) fn check_new_name(path: &Path) -> io::Result<()> {
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = name.to_string_lossy();
        if win32_name_issue(&name) == Some(Win32NameIssue::InvalidCharacter) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidFilename,
                format!(
                    "Name auf Windows-Ziel nicht möglich ({}): {name}",
                    Win32NameIssue::InvalidCharacter.label_de()
                ),
            ));
        }
    }
    Ok(())
}

/// A new file for a stage (it inherits its folder's ACL like any file).
pub(crate) fn create_new_private(path: &Path) -> io::Result<std::fs::File> {
    check_new_name(path)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Pin and classify the stage itself before requesting write access. A
/// kernel copy may have copied the read-only bit: temporarily lifting it
/// grants a writable handle, then the original bit is restored immediately.
pub(crate) fn open_stage(path: &Path) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_WRITE_ATTRIBUTES,
    };
    let path = crate::local_access::normalize_scan_root(path);
    let guard = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)?;
    let class = crate::local_access::classify_open_file(&guard)?;
    if class.link_like {
        return Err(crate::local_access::NotRegular::Link.error());
    }
    if class.special || !guard.metadata()?.is_file() {
        return Err(crate::local_access::NotRegular::Special.error());
    }
    let permissions = guard.metadata()?.permissions();
    let read_only = permissions.readonly();
    if read_only {
        let mut writable = permissions.clone();
        writable.set_readonly(false);
        guard.set_permissions(writable)?;
    }
    let result = std::fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path);
    if read_only {
        guard.set_permissions(permissions)?;
    }
    let file = result?;
    let class = crate::local_access::classify_open_file(&file)?;
    if class.link_like || class.special || !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stage changed while opening",
        ));
    }
    Ok(file)
}

fn move_replacing(source: &[u16], destination: &[u16]) -> io::Result<()> {
    // SAFETY: both are NUL-terminated wide strings that outlive the call.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Atomically replace `destination` (a regular file the caller checked) with
/// `source`, written through to disk. Windows refuses to replace a read-only
/// file: its attribute is lifted for the replacement and set on the new
/// file, or put back when the replacement still fails.
pub(crate) fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source);
    let destination = wide(destination);
    let error = match move_replacing(&source, &destination) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if error.raw_os_error() != Some(ERROR_ACCESS_DENIED as i32) {
        return Err(error);
    }
    // SAFETY: NUL-terminated wide string that outlives the call.
    let attributes = unsafe { GetFileAttributesW(destination.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES
        || attributes & FILE_ATTRIBUTE_READONLY == 0
        || attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    {
        return Err(error);
    }
    // SAFETY: as above.
    if unsafe { SetFileAttributesW(destination.as_ptr(), attributes & !FILE_ATTRIBUTE_READONLY) }
        == 0
    {
        return Err(error);
    }
    if let Err(retry) = move_replacing(&source, &destination) {
        // SAFETY: as above; best effort to leave the old file as it was.
        unsafe { SetFileAttributesW(destination.as_ptr(), attributes) };
        return Err(retry);
    }
    // SAFETY: as above. The content is published; the attribute is metadata
    // and a failure to set it is not a failed replacement.
    unsafe {
        let published = GetFileAttributesW(destination.as_ptr());
        if published != INVALID_FILE_ATTRIBUTES {
            SetFileAttributesW(destination.as_ptr(), published | FILE_ATTRIBUTE_READONLY);
        }
    }
    Ok(())
}
