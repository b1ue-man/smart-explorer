use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) fn local_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

pub(crate) fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

/// The OS path for a forward-slash VFS path; Unix names need no rewriting.
pub(crate) fn to_os(path: &str) -> PathBuf {
    PathBuf::from(path)
}

/// Key of the volume serving `path` for shared concurrency control: the
/// device of the nearest existing ancestor, so separate disks are separate.
pub(crate) fn volume_key(path: &str) -> String {
    use std::os::unix::fs::MetadataExt;
    let mut current = Path::new(path);
    loop {
        if let Ok(metadata) = std::fs::metadata(current) {
            return format!("local:dev{}", metadata.dev());
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return "local:".to_string(),
        }
    }
}

pub(crate) fn reported_name(path: &Path) -> Option<OsString> {
    path.file_name().map(OsString::from)
}

pub(crate) fn remove_file_like(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// `syncfs(2)` on the filesystem holding `path`: afterwards everything
/// written there before is on stable storage, as if each file had been
/// fsynced (Linux ≥ 2.6.39, bionic API 28).
pub(crate) fn syncfs(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    // SAFETY: the descriptor belongs to `handle`, which outlives the call.
    if unsafe { libc::syncfs(handle.as_raw_fd()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(true)
}

/// Permission bits (with set-id and sticky bits) of a local entry.
pub(crate) fn unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o7777)
}

/// `fchmod` on an open file; FAT mounts refuse it, Android storage ignores it.
pub(crate) fn set_unix_mode(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

/// Filesystem identity of `path` (filesystem UUID + location inside it).
/// `Ok(None)` = not determinable here, which callers treat as "unknown",
/// never as "another volume".
pub(crate) fn volume_identity(_path: &Path) -> std::io::Result<Option<super::VolumeIdentity>> {
    Ok(None)
}

/// Kind of filesystem mounted at `path` when it is a mount point inside its
/// parent's tree (`None` = same filesystem as the parent, or not known).
pub(crate) fn mount_boundary(_path: &Path) -> std::io::Result<Option<super::MountKind>> {
    Ok(None)
}

#[cfg(target_os = "linux")]
pub(crate) fn rename_no_replace(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let source = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source path contains NUL")
    })?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination path contains NUL",
        )
    })?;
    // The libc crate omits the renameat2 wrapper on musl. Use the Linux
    // syscall directly and propagate ENOSYS rather than weakening the atomic
    // no-replace contract with a check-then-rename fallback.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Android storage may lack `RENAME_NOREPLACE` and hard links; the fallback
/// chain lives in `android_fs`.
#[cfg(target_os = "android")]
pub(crate) fn rename_no_replace(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    crate::android_fs::rename_no_replace(source, destination)
}
