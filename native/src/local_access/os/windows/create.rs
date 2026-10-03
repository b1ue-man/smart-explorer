//! Private Win32 children are created with a protected owner DACL before
//! any bytes are written. Pinned ancestors keep the OS path confined.
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::sync::Arc;
use std::os::windows::fs::OpenOptionsExt;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, READ_CONTROL, WRITE_DAC,
};

use super::{validate_directory, validate_name, DirectoryHandle, PinnedDirectory};

#[path = "private_security.rs"]
mod private_security;
pub(crate) use private_security::secure_private_handle;

impl DirectoryHandle {
    pub(crate) fn secure_private(&self) -> io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&self.0.path)?;
        if !super::identity::same_object(&self.0.file, &file)? {
            return Err(io::Error::other("private directory identity changed"));
        }
        secure_private_handle(&file, true)
    }

    pub(crate) fn create_private_child(&self, name: &OsStr) -> io::Result<Self> {
        validate_name(name)?;
        let path = self.0.path.join(name);
        let file = private_security::create_directory(&path)?;
        validate_directory(&file)?;
        Ok(Self(Arc::new(PinnedDirectory {
            file,
            path,
            _parent: Some(self.0.clone()),
            _ancestors: Vec::new(),
            consented_path: None,
        })))
    }

    pub(crate) fn create_file_new(&self, name: &OsStr) -> io::Result<File> {
        validate_name(name)?;
        private_security::create_file(&self.0.path.join(name))
    }
}
