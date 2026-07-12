use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) fn local_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

pub(crate) fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

pub(crate) fn to_os(path: &str) -> PathBuf {
    PathBuf::from(path)
}

pub(crate) fn reported_name(path: &Path) -> Option<OsString> {
    path.file_name().map(OsString::from)
}

pub(crate) fn remove_file_like(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

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
