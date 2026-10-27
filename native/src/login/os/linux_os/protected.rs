//! openat/no-follow ancestry and root ownership protect administrative trust.
use crate::login::os::shared::denied;
use std::{
    ffi::CString,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};
pub(crate) struct ProtectedFile {
    pub file: File,
    _parents: Vec<File>,
}
pub(crate) fn open_protected(path: &Path, executable: bool) -> io::Result<ProtectedFile> {
    open_owned(path, 0, executable)
}
pub(super) fn open_owned(
    path: &Path,
    allowed_uid: u32,
    executable: bool,
) -> io::Result<ProtectedFile> {
    open_with(path, allowed_uid, None, executable)
}
/// A greeter credential: as `open_owned`, but a directory or file owned by
/// `uid` may also be writable by `gid`, that account's own primary group,
/// when the group lists no members. LightDM ships `/var/lib/lightdm` 0770 and
/// the greeter's `.Xauthority` 0660, both `lightdm:lightdm`. Assumption: no
/// other account uses `gid` as its primary group (not checked).
pub(super) fn open_greeter_owned(path: &Path, uid: u32, gid: u32) -> io::Result<ProtectedFile> {
    let group = (uid != 0 && group_without_members(gid)).then_some(gid);
    open_with(path, uid, group, false)
}
fn group_without_members(gid: u32) -> bool {
    let mut record: libc::group = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let mut bytes = vec![0u8; 64 * 1024];
    // SAFETY: record, buffer and result outlive the call; result is checked.
    let code = unsafe {
        libc::getgrgid_r(
            gid,
            &mut record,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut result,
        )
    };
    // SAFETY: gr_mem is the NULL-terminated member list inside `bytes`.
    code == 0
        && !result.is_null()
        && (record.gr_mem.is_null() || unsafe { (*record.gr_mem).is_null() })
}
fn open_with(
    path: &Path,
    allowed_uid: u32,
    group: Option<u32>,
    executable: bool,
) -> io::Result<ProtectedFile> {
    if !path.is_absolute() {
        return Err(denied("login trust path must be absolute"));
    }
    let mut parents = vec![File::open("/")?];
    check(&parents[0], true, allowed_uid, group, false)?;
    let parts: Vec<_> = path
        .components()
        .filter(|part| !matches!(part, Component::RootDir))
        .collect();
    if parts.is_empty() {
        return Err(denied("login trust path has no file"));
    }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(denied("login trust path contains traversal"));
        };
        let name = CString::new(name.as_bytes()).map_err(io::Error::other)?;
        let directory = index + 1 != parts.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        // SAFETY: retained parent FD, one NUL-terminated component and checked result.
        let fd = unsafe {
            libc::openat(
                parents
                    .last()
                    .ok_or_else(|| denied("missing login parent pin"))?
                    .as_raw_fd(),
                name.as_ptr(),
                flags,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        check(
            &file,
            directory,
            allowed_uid,
            group,
            executable && !directory,
        )?;
        if directory {
            parents.push(file);
        } else {
            return Ok(ProtectedFile {
                file,
                _parents: parents,
            });
        }
    }
    Err(denied("login trust file missing"))
}
/// Write bits nobody but the owner (or the tolerated private group of an
/// object `uid` owns) may hold.
fn foreign_write(mode: u32, owner: u32, owner_group: u32, uid: u32, group: Option<u32>) -> bool {
    let tolerated = match group {
        Some(gid) if uid != 0 && owner == uid && owner_group == gid => 0o020,
        _ => 0,
    };
    mode & 0o022 & !tolerated != 0
}
fn check(
    file: &File,
    directory: bool,
    uid: u32,
    group: Option<u32>,
    executable: bool,
) -> io::Result<()> {
    let meta = file.metadata()?;
    if (meta.uid() != 0 && meta.uid() != uid)
        || foreign_write(meta.mode(), meta.uid(), meta.gid(), uid, group)
        || (directory && !meta.is_dir())
        || (!directory && (!meta.is_file() || meta.nlink() != 1))
        || (executable && (meta.mode() & 0o111 == 0 || meta.mode() & 0o6000 != 0))
    {
        return Err(denied(
            "login trust path has unsafe owner, mode, link count or file kind",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_metadata_runtime_task_login_greeter_private_group_write() {
        // LightDM: /var/lib/lightdm 0770 and .Xauthority 0660, lightdm:lightdm.
        assert!(!foreign_write(0o40770, 113, 117, 113, Some(117)));
        assert!(!foreign_write(0o100660, 113, 117, 113, Some(117)));
        assert!(foreign_write(0o100660, 113, 117, 113, None));
        assert!(foreign_write(0o100662, 113, 117, 113, Some(117)));
        assert!(foreign_write(0o100660, 113, 118, 113, Some(117)));
        assert!(foreign_write(0o100660, 0, 117, 113, Some(117)));
        assert!(foreign_write(0o100660, 0, 0, 0, Some(0)));
        assert!(!foreign_write(0o40755, 0, 0, 113, Some(117)));
    }
}
