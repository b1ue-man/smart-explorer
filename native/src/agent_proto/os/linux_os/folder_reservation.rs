//! Plain-directory identity from the held descriptor, independent of name.
use std::{
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(crate) fn identity(path: &Path) -> io::Result<Option<String>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| {
            if path.has_root() {
                path
            } else {
                Path::new(".")
            }
        });
    super::super::promotion::validate_destination_root(parent)?;
    let directory = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(directory) => directory,
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)) => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Ok(None);
    }
    let birth = metadata
        .created()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| format!(":{}:{}", time.as_secs(), time.subsec_nanos()))
        .unwrap_or_default();
    Ok(Some(format!(
        "agent-directory-unix:{}:{}{birth}",
        metadata.dev(),
        metadata.ino()
    )))
}
