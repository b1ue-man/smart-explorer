//! Windows publication uses the same write-through boundary as native
//! stages. Held private parents prevent a link/junction ancestor swap.
use crate::local_access::DirectoryHandle;
use std::{io, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub(crate) fn publish(source: &Path, target: &Path, replace: bool) -> io::Result<()> {
    let parent = source
        .parent()
        .ok_or_else(|| io::Error::other("Recoveryquelle hat keinen Parent"))?;
    if target.parent() != Some(parent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recovery-Publish überschreitet den Parent",
        ));
    }
    let root = DirectoryHandle::open_root(parent)?;
    confirm_namespace(parent)?;
    let source_meta = crate::local_access::symlink_metadata(source)?;
    if crate::local_access::metadata_is_link_like(source, &source_meta) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Recoveryquelle ist ein umleitender Link",
        ));
    }
    match crate::local_access::symlink_metadata(target) {
        Ok(meta)
            if !replace
                || !meta.is_file()
                || crate::local_access::metadata_is_link_like(target, &meta) =>
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Recoveryziel ist belegt oder kein regulärer Datensatz",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: terminated immutable native strings and private pinned parent.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Qualification only; durability is provided by publish's write-through
/// rename after every payload/manifest file has been synced.
pub(crate) fn confirm_namespace(path: &Path) -> io::Result<()> {
    let spelling = path
        .to_str()
        .ok_or_else(|| io::Error::other("Recoverypfad ist nicht darstellbar"))?;
    if !crate::vfs::confirm_namespace(&crate::vfs::LocalBackend::new(spelling), spelling)? {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Dateisystem unterstützt keine bestätigte Recovery-Veröffentlichung",
        ));
    }
    Ok(())
}
