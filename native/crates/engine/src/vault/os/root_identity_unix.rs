//! Identity of the selected local root; this is an identity check, not confinement.
use std::io;
use std::os::unix::fs::MetadataExt;

pub(super) fn identity(root: &str) -> io::Result<String> {
    let held = crate::local_access::DirectoryHandle::open_root(std::path::Path::new(root))?;
    let meta = held.metadata()?;
    Ok(format!("unix:{}:{}", meta.dev(), meta.ino()))
}
