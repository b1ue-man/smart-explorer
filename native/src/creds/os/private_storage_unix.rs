//! Private application objects. Creation sets restrictive mode bits before
//! publication; validation and migration operate on the opened object.
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path};

fn component(name: &std::ffi::OsStr) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in private path"))
}

fn directory(path: &Path, create: bool) -> io::Result<File> {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let mut names = Vec::new();
    for part in absolute.components() {
        match part {
            Component::Normal(name) => names.push(name),
            Component::ParentDir => { names.pop(); },
            Component::RootDir | Component::CurDir => {},
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Unsupported private root")),
        }
    }
    if names.is_empty() { return Err(io::Error::other("Filesystem root is not private application storage")); }
    let mut logical = std::path::PathBuf::from("/");
    for name in &names { logical.push(name); }
    if std::fs::symlink_metadata(&logical).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Private directory must not be a link"));
    }
    let parent = logical.parent().ok_or_else(|| io::Error::other("Private directory needs a parent"))?;
    if create && !parent.exists() { ensure_directory(parent)?; }
    // Select the physical parent once (including OS data-root aliases, e.g.
    // Android filesDir), then pin it without following child redirects.
    let selected = std::fs::canonicalize(parent)?.join(names.last().ok_or_else(|| io::Error::other("Missing private name"))?);
    let names: Vec<_> = selected.components().filter_map(|part| match part {
        Component::Normal(name) => Some(name), _ => None,
    }).collect();
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::open(c"/".as_ptr(), flags) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let mut parent = unsafe { File::from_raw_fd(fd) };
    for name in names {
        let name = component(name)?;
        let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
            if made < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
            fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        }
        if fd < 0 { return Err(io::Error::last_os_error()); }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    secure(&parent, true)?;
    Ok(parent)
}

pub(crate) fn ensure_directory(path: &Path) -> io::Result<()> { directory(path, true).map(drop) }

pub(crate) fn create_file(path: &Path) -> io::Result<File> { open(path, true, true) }
pub(crate) fn open_file(path: &Path, writable: bool) -> io::Result<File> { open(path, writable, false) }

fn open(path: &Path, writable: bool, create: bool) -> io::Result<File> {
    let parent = path.parent().ok_or_else(|| io::Error::other("Private file needs a parent"))?;
    let name = path.file_name().ok_or_else(|| io::Error::other("Private file needs a name"))?;
    let parent = directory(parent, false)?;
    let name = component(name)?;
    let flags = (if writable { libc::O_RDWR } else { libc::O_RDONLY })
        | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK
        | if create { libc::O_CREAT | libc::O_EXCL } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let file = unsafe { File::from_raw_fd(fd) };
    secure(&file, false)?;
    Ok(file)
}

pub(crate) fn secure(file: &File, directory: bool) -> io::Result<()> {
    let metadata = file.metadata()?;
    let uid = unsafe { libc::geteuid() };
    let kind_matches = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file() && metadata.nlink() == 1
    };
    if metadata.uid() != uid || !kind_matches {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private application object has the wrong owner, kind or link count",
        ));
    }
    let mode = if directory { 0o700 } else { 0o600 };
    if metadata.mode() & 0o7777 != mode {
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    let verified = file.metadata()?;
    if verified.uid() != uid || verified.mode() & 0o7777 != mode
        || (!directory && verified.nlink() != 1)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private application permissions could not be retained",
        ));
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> { directory(path, false)?.sync_all() }

#[cfg(test)]
#[path = "private_storage_tests.rs"]
mod tests;
