use super::{broker, normalize_scan_root, privilege::BackupRead};
use crate::local_access::protocol::{self, ReadKind};
use std::{
    fs::{File, Metadata, OpenOptions},
    io,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::*;

fn access(kind: ReadKind) -> u32 {
    match kind {
        ReadKind::Metadata => FILE_READ_ATTRIBUTES,
        ReadKind::Directory => FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
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

fn open_direct(path: &Path, kind: ReadKind, sharing: u32) -> io::Result<File> {
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
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Lesequelle ist keine direkte reguläre Datei",
        ));
    }
    Ok(file)
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

/// The helper pins every ancestor while opening the leaf. A rename or reparse
/// replacement cannot redirect a consented path outside the authorized root.
pub(super) fn open_scoped(root: &str, path: &str, kind: ReadKind) -> io::Result<File> {
    if !protocol::contains(root, path) {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    let _backup = BackupRead::enable()?;
    let target = normalize_scan_root(Path::new(path));
    let mut held = Vec::new();
    let mut current = PathBuf::new();
    for component in target.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) || current == target {
            continue;
        }
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let file = open_direct(&current, ReadKind::Metadata, FILE_SHARE_READ)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        held.push(file);
    }
    let file = open_direct(&target, kind, FILE_SHARE_READ | FILE_SHARE_WRITE)?;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (matches!(kind, ReadKind::Directory) && !metadata.is_dir())
        || (matches!(kind, ReadKind::File) && !metadata.is_file())
    {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(file)
}
