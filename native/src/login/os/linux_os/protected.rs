//! openat/no-follow ancestry and root ownership protect administrative trust.
use std::{ffi::CString, fs::File, io, os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::MetadataExt}}, path::{Component, Path}};
use crate::login::os::shared::denied;
pub(crate) struct ProtectedFile { pub file: File, _parents: Vec<File> }
pub(crate) fn open_protected(path: &Path, executable: bool) -> io::Result<ProtectedFile> {
    open_owned(path, 0, executable)
}
pub(super) fn open_owned(path: &Path, allowed_uid: u32, executable: bool) -> io::Result<ProtectedFile> {
    if !path.is_absolute() { return Err(denied("login trust path must be absolute")); }
    let mut parents = vec![File::open("/")?];
    check(&parents[0], true, allowed_uid, false)?;
    let parts: Vec<_> = path.components().filter(|part| !matches!(part, Component::RootDir)).collect();
    if parts.is_empty() { return Err(denied("login trust path has no file")); }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else { return Err(denied("login trust path contains traversal")); };
        let name = CString::new(name.as_bytes()).map_err(io::Error::other)?;
        let directory = index + 1 != parts.len();
        let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        // SAFETY: retained parent FD, one NUL-terminated component and checked result.
        let fd = unsafe { libc::openat(parents.last().ok_or_else(|| denied("missing login parent pin"))?.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_fd(fd) };
        check(&file, directory, allowed_uid, executable && !directory)?;
        if directory { parents.push(file); } else { return Ok(ProtectedFile { file, _parents: parents }); }
    }
    Err(denied("login trust file missing"))
}
fn check(file: &File, directory: bool, uid: u32, executable: bool) -> io::Result<()> {
    let meta = file.metadata()?;
    if (meta.uid() != 0 && meta.uid() != uid) || meta.mode() & 0o022 != 0
        || (directory && !meta.is_dir()) || (!directory && (!meta.is_file() || meta.nlink() != 1))
        || (executable && (meta.mode() & 0o111 == 0 || meta.mode() & 0o6000 != 0)) {
        return Err(denied("login trust path has unsafe owner, mode, link count or file kind"));
    }
    Ok(())
}
