//! Reopening a closed Unix stage never waits on or follows a replaced leaf.
use std::{fs::File, io, os::unix::fs::OpenOptionsExt, path::Path};

use crate::local_access::NotRegular;

pub(crate) fn open_stage(path: &Path) -> io::Result<File> {
    let before = std::fs::symlink_metadata(path)?;
    if before.is_symlink() {
        return Err(NotRegular::Link.error());
    }
    if before.is_dir() {
        return Err(NotRegular::Directory.error());
    }
    if !before.is_file() {
        return Err(NotRegular::Special.error());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) => NotRegular::Link.error(),
            Some(libc::ENXIO) => NotRegular::Special.error(),
            _ => error,
        })?;
    if !file.metadata()?.is_file() {
        return Err(NotRegular::Special.error());
    }
    Ok(file)
}
