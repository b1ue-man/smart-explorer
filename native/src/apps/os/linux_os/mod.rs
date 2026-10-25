mod catalog;
mod desktop_entry;
mod launch;
mod session;
mod window_correlation;
pub(crate) use launch::Platform;

use std::{fs, io, path::Path};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub(crate) fn prepare_journal(root: &Path) -> io::Result<()> {
    let uid=unsafe {libc::geteuid()};
    if !root.exists() { fs::DirBuilder::new().recursive(true).create(root)?; fs::set_permissions(root,fs::Permissions::from_mode(0o700))?; }
    let metadata=fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid()!=uid || metadata.mode()&0o077!=0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,"launch ledger must be a private directory owned by the interactive user"));
    }
    Ok(())
}
pub(crate) fn sync_journal(root: &Path) -> io::Result<()> {fs::File::open(root)?.sync_all()}
pub(crate) fn publish_record(file:tempfile::NamedTempFile,path:&Path,replace:bool)->io::Result<bool> {
    if replace {file.persist(path).map_err(|e|e.error)?;return Ok(true);}
    match file.persist_noclobber(path) {Ok(_)=>Ok(true),Err(e) if e.error.kind()==io::ErrorKind::AlreadyExists=>Ok(false),Err(e)=>Err(e.error)}
}
