//! Durable publication beneath one pinned, private parent.
use crate::local_access::DirectoryHandle;
use std::{
    ffi::{CString, OsStr},
    io,
    os::unix::{ffi::OsStrExt, io::AsRawFd},
    path::Path,
};

pub(crate) fn publish(source: &Path, target: &Path, replace: bool) -> io::Result<()> {
    let parent = source
        .parent()
        .ok_or_else(|| io::Error::other("Recoveryquelle hat keinen Parent"))?;
    if target.parent() != Some(parent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recovery-Publish überschreitet den Parent",
        ));
    }
    let root = DirectoryHandle::open_root(parent)?;
    let source_name = CString::new(source.file_name().unwrap_or(OsStr::new("")).as_bytes())
        .map_err(io::Error::other)?;
    let target_name = CString::new(target.file_name().unwrap_or(OsStr::new("")).as_bytes())
        .map_err(io::Error::other)?;
    let fd = root.directory_file().as_raw_fd();
    // SAFETY: both names and the same held parent descriptor stay alive.
    let result = unsafe {
        libc::renameat2(
            fd,
            source_name.as_ptr(),
            fd,
            target_name.as_ptr(),
            if replace { 0 } else { libc::RENAME_NOREPLACE },
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    root.directory_file().sync_all()
}

pub(crate) fn confirm_namespace(path: &Path) -> io::Result<()> {
    DirectoryHandle::open_root(path)?
        .directory_file()
        .sync_all()
}
