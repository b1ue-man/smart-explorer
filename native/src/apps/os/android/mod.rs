pub(crate) mod bridge;
mod launch;
pub(crate) use launch::Platform;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{fs, io, path::Path};
pub(crate) fn prepare_journal(root: &Path) -> io::Result<()> {
    if !root.exists() {
        fs::create_dir_all(root)?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    }
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Android launch ledger must belong to the app's private UID",
        ));
    }
    Ok(())
}
pub(crate) fn sync_journal(root: &Path) -> io::Result<()> {
    fs::File::open(root)?.sync_all()
}
pub(crate) fn publish_record(
    file: tempfile::NamedTempFile,
    path: &Path,
    replace: bool,
) -> io::Result<bool> {
    if replace {
        file.persist(path).map_err(|e| e.error)?;
        return Ok(true);
    }
    match file.persist_noclobber(path) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.error),
    }
}
