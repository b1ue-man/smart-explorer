//! Replacing a local file on Windows. Below vfs and support_dirs (plan
//! docs/plaene/2026-11-01-idealzustand, D13); vfs re-exports it.
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, MoveFileExW, SetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_READONLY, INVALID_FILE_ATTRIBUTES, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH,
};

/// A path as the NUL-terminated wide string the Win32 calls take, normalised
/// like scan roots (verbatim prefix for long paths).
pub fn wide(path: &Path) -> Vec<u16> {
    crate::local_access::normalize_scan_root(path)
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect()
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
pub fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
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
