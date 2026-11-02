//! Worker log file on Unix: created 0600 and never opened through a link.
pub fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "log must be a regular file",
        ));
    }
    Ok(file)
}
