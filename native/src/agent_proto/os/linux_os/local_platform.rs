use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

pub(crate) type FileIdentity = (u64, u64);

pub(crate) fn metadata_is_link_like(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// FIFO, socket or device: no data stream to read.
pub(crate) fn metadata_is_special(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let kind = metadata.file_type();
    kind.is_fifo() || kind.is_socket() || kind.is_block_device() || kind.is_char_device()
}

pub(crate) fn metadata_class(path: &Path, metadata: &std::fs::Metadata) -> (bool, bool) {
    (
        metadata_is_link_like(path, metadata),
        metadata_is_special(metadata),
    )
}

/// The checked descriptor, not a second path lookup, is used by hash walks
/// and stage finishing. A replaced leaf link is refused and a FIFO cannot wait.
pub(crate) fn open_regular_no_follow(path: &Path, write: bool) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(!write)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "keine reguläre Datei",
        ));
    }
    Ok(file)
}

pub(crate) fn set_file_mode(file: &std::fs::File, mode: u32) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))
}

/// Opens `path` for reading only when it is a regular file (links are
/// followed as before): the open never waits on a FIFO and a device or
/// socket is refused before any byte is read.
pub(crate) fn open_regular_file(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("keine reguläre Datei: {}", path.display()),
        ));
    }
    // Back to ordinary blocking reads for the regular file.
    let fd = file.as_raw_fd();
    // SAFETY: `fd` belongs to `file`, which stays open across both calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

/// Flushes the filesystem that holds `path` (`syncfs`); `false` where the
/// call is not available.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn sync_filesystem(path: &Path) -> io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    let directory = std::fs::File::open(path)?;
    // SAFETY: the descriptor stays open for the duration of the call.
    if unsafe { libc::syncfs(directory.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(true)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(crate) fn sync_filesystem(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

pub(crate) fn file_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

pub(crate) fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(file_identity(&file)? == expected)
}

pub(crate) fn secure_staging_directory(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

pub(crate) fn secure_staging_file(file: &std::fs::File) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

pub(crate) fn replace_file_atomic(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

/// Atomically move `source` to a name that must not already exist.
#[cfg(target_os = "linux")]
pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    // `libc` does not expose the renameat2 wrapper on musl targets. Invoke the
    // Linux syscall directly so this atomic no-replace primitive is available
    // to both glibc and static agent builds. Unsupported calls use an
    // exclusive hard link below; no existence-check + rename can race.
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
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        return Err(error);
    }
    // No check-then-rename: on filesystems without rename2 a hard link also
    // claims the destination exclusively. Directories and filesystems without
    // links retain the source and fail safely; the standalone agent needs no
    // dependency on the application's android_fs module for this ladder.
    let source_path = Path::new(std::ffi::OsStr::from_bytes(source.as_bytes()));
    let destination_path = Path::new(std::ffi::OsStr::from_bytes(destination.as_bytes()));
    hard_link_no_replace(source_path, destination_path)
}

#[cfg(target_os = "linux")]
fn hard_link_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let opened = open_regular_no_follow(source, false)?;
    let expected = file_identity(&opened)?;
    std::fs::hard_link(source, destination)?;
    let published = open_regular_no_follow(destination, false)?;
    let remaining = open_regular_no_follow(source, false)?;
    if file_identity(&published)? != expected || file_identity(&remaining)? != expected {
        // Keep both names for recovery; never unlink a substituted source.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Stage während Veröffentlichung geändert",
        ));
    }
    std::fs::remove_file(source)
}

/// Android storage may lack `RENAME_NOREPLACE` and hard links; the fallback
/// chain lives in `android_fs`. The standalone `se-agent` build never targets
/// Android, so this arm only builds inside the app crate.
#[cfg(target_os = "android")]
pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    crate::android_fs::rename_no_replace(source, destination)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn review_task_agent_regular_handles_refuse_leaf_links_and_fifos() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        let link = dir.path().join("link");
        let fifo = dir.path().join("fifo");
        std::fs::write(&file, b"original").unwrap();
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let c_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        assert!(open_regular_no_follow(&fifo, false).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(open_regular_no_follow(&link, false).is_err());
        assert!(open_regular_no_follow(&link, true).is_err());
        let handle = open_regular_no_follow(&file, true).unwrap();
        std::fs::rename(&file, dir.path().join("opened")).unwrap();
        std::fs::write(&file, b"replacement").unwrap();
        set_file_mode(&handle, 0o400).unwrap();
        assert_eq!(
            std::fs::metadata(dir.path().join("opened"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        assert_ne!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o400
        );
    }

    #[test]
    fn review_task_agent_noreplace_hardlink_fallback_preserves_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("stage");
        let target = dir.path().join("target");
        std::fs::write(&stage, b"candidate").unwrap();
        std::fs::write(&target, b"existing").unwrap();
        assert_eq!(
            hard_link_no_replace(&stage, &target).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&stage).unwrap(), b"candidate");
        assert_eq!(std::fs::read(&target).unwrap(), b"existing");
        std::fs::remove_file(&target).unwrap();
        hard_link_no_replace(&stage, &target).unwrap();
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"candidate");
    }
}
