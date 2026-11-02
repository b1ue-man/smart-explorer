//! Worker log file on Windows: a reparse point is opened, never followed.
pub fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "log must be a regular file",
        ));
    }
    Ok(file)
}
