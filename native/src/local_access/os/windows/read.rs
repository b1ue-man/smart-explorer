use super::{broker, normalize_scan_root, privilege::BackupRead};
use crate::local_access::protocol::ReadKind;
use crate::local_access::FinalLink;
use std::{
    fs::{File, Metadata, OpenOptions},
    io,
    os::windows::fs::OpenOptionsExt,
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::*;

fn access(kind: ReadKind) -> u32 {
    match kind {
        ReadKind::Metadata => FILE_READ_ATTRIBUTES,
        ReadKind::Directory | ReadKind::PinRoot | ReadKind::PinChild => {
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES
        }
        ReadKind::File => FILE_GENERIC_READ,
    }
}

pub(super) fn open(path: &Path, kind: ReadKind) -> io::Result<File> {
    let path = normalize_scan_root(path);
    let attempt = || {
        open_direct(
            &path,
            kind,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
    };
    match attempt() {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            if let Ok(_backup) = BackupRead::enable() {
                return attempt();
            }
            // A caller's explicit impersonation must never be replaced by a
            // session grant belonging to the ordinary GUI process.
            if super::privilege::parallel_scan_allowed() {
                broker::open_granted(&path, kind).unwrap_or(Err(error))
            } else {
                Err(error)
            }
        }
        result => result,
    }
}

pub(super) fn open_direct(path: &Path, kind: ReadKind, sharing: u32) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(access(kind))
        .share_mode(sharing)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    match File::open(normalize_scan_root(path)) {
        Ok(file) => return Ok(file),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
        Err(error) => return Err(error),
    }
    let file = open(path, ReadKind::File)?;
    super::regular::validate_file(&file, true)?;
    Ok(file)
}

/// With `FinalLink::Refuse` the entry itself is classified first (one
/// reparse-tag read), so a link is refused instead of followed; the opened
/// handle must then be a regular file. Data reparse points (cloud
/// placeholders, WOF, dedup) are regular files and open as usual.
pub(crate) fn open_regular(path: &Path, final_link: FinalLink) -> io::Result<File> {
    super::regular::open_regular(path, final_link)
}

pub(crate) fn symlink_metadata(path: &Path) -> io::Result<Metadata> {
    let path = normalize_scan_root(path);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            open(&path, ReadKind::Metadata)?.metadata()
        }
        result => result,
    }
}
