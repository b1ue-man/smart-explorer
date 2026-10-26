//! File object evidence on the standalone agent's existing local OS boundary.
//! This module also builds in se-agent, which has no application VFS/copy module.
use std::{io, path::Path};

pub(crate) fn identity(path: &str) -> io::Result<Option<String>> {
    let path = Path::new(path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        super::promotion::validate_destination_root(parent)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || super::local_platform::metadata_is_link_like(path, &metadata) {
        return Ok(None);
    }
    let file = super::local_platform::open_regular_no_follow(path, false)?;
    let identity = match super::local_platform::stable_handle_identity(&file) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error),
    };
    // Reopen without following the leaf, like the guarded transfer path.
    // A replacement between the metadata read and this check is not ours.
    let checked = super::local_platform::open_regular_no_follow(path, false)?;
    if !super::local_platform::same_file_object(&checked, &file)?
        || super::local_platform::stable_handle_identity(&checked)? != identity
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "File object changed during identity query",
        ));
    }
    Ok(Some(format!("agent-local-file-v2:{identity}")))
}
