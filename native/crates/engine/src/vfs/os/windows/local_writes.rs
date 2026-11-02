//! Creating and replacing local files on Windows: names NTFS would turn into
//! alternate data streams are refused, and a read-only destination is
//! replaced (its attribute moves to the new file).
use std::io;
use std::path::{Component, Path};

use crate::host::wide_local_path as wide;
use crate::types::{win32_name_issue, Win32NameIssue};

/// Refuses to create a name Win32 rejects: `:` would write into an
/// alternate data stream of another file, `? * " < > |` and control
/// characters fail on NTFS anyway. Existing reserved names and names with a
/// trailing dot or space stay reachable through verbatim paths.
pub fn check_new_name(path: &Path) -> io::Result<()> {
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
pub fn create_new_private(path: &Path) -> io::Result<std::fs::File> {
    check_new_name(path)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Pin and classify the stage itself before requesting write access. A
/// kernel copy may have copied the read-only bit: temporarily lifting it
/// grants a writable handle, then the original bit is restored immediately.
pub fn open_stage(path: &Path) -> io::Result<std::fs::File> {
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
