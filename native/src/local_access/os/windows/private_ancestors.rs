//! Physical object identity also covers UNC aliases of a private directory.
use std::{io, path::PathBuf};
use std::os::windows::fs::OpenOptionsExt;
use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE};
use super::{identity, DirectoryHandle};

impl DirectoryHandle {
    pub(crate) fn is_within_any(&self, roots: &[PathBuf]) -> io::Result<bool> {
        let ancestors = self.pin_files();
        for path in roots {
            // A trusted private-root name needs only object identity. Do not
            // canonicalize/pin its ancestors again for every entered child.
            let root = match std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path) {
                Ok(root) => root,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for ancestor in &ancestors {
                // The existing typed identity compares volume + full 128-bit
                // FileId; its older-volume fallback rejects unknown identity.
                if identity::same_object(ancestor, &root)? { return Ok(true); }
            }
        }
        Ok(false)
    }
}
