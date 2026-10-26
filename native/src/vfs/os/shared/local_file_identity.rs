//! File identities through the same retained authority as local VFS I/O.
use std::{fs::File, io};

pub(super) fn local_file_identity(path: &str) -> io::Result<Option<String>> {
    let path = super::local_platform::to_os(path);
    let _permit =
        crate::source_media::acquire_current(&path, crate::source_media::Priority::Foreground)?;
    let Some(name) = path.file_name() else {
        return Ok(None);
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let parent = crate::local_access::ConfinedDirectory::open_root(parent)?;
    handle_identity(&parent.selected_entry(name)?)
}

pub(super) fn handle_identity(file: &File) -> io::Result<Option<String>> {
    let metadata = file.metadata()?;
    let class = crate::local_access::classify_open_file(file)?;
    if !metadata.is_file() || class.link_like || class.special {
        return Ok(None);
    }
    match crate::copy::local_file_identity_handle(file) {
        Ok(identity) => Ok(Some(identity)),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(None),
        Err(error) => Err(error),
    }
}
