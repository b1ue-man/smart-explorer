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
    // to both glibc and the static agent builds. ENOSYS is intentionally
    // returned to the caller: an existence-check + rename fallback would race.
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
        Err(io::Error::last_os_error())
    }
}

/// Android storage may lack `RENAME_NOREPLACE` and hard links; the fallback
/// chain lives in `android_fs`. The standalone `se-agent` build never targets
/// Android, so this arm only builds inside the app crate.
#[cfg(target_os = "android")]
pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    crate::android_fs::rename_no_replace(source, destination)
}
