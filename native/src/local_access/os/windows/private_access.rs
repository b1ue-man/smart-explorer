//! Private contents open under the retained directory ancestry. Only a file
//! handle escapes; a synchronous read pin never supplies a watch capability.
use std::{ffi::OsStr, fs::File, io, os::windows::fs::OpenOptionsExt};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE},
    Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        READ_CONTROL, WRITE_DAC,
    },
};

use super::{secure_private_handle, validate_name, DirectoryHandle};

impl DirectoryHandle {
    /// Validate the pinned parent and opened owned object before returning
    /// any contents. The leaf refuses redirects, specials and other hardlinks.
    /// Writable private callers retain their existing write-sharing contract.
    pub(crate) fn open_private_child(&self, name: &OsStr, writable: bool) -> io::Result<File> {
        validate_name(name)?;
        self.secure_private()?;
        let access = GENERIC_READ
            | FILE_READ_ATTRIBUTES
            | READ_CONTROL
            | WRITE_DAC
            | if writable { GENERIC_WRITE } else { 0 };
        let file = std::fs::OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | if writable { FILE_SHARE_WRITE } else { 0 })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path().join(name))?;
        secure_private_handle(&file, false)?;
        Ok(file)
    }
}
