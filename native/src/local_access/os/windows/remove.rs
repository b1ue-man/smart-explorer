//! DELETE access belongs to the original pin; never drop it to reopen a path.
use std::{ffi::OsStr, fs::File, io, mem::size_of, os::windows::{fs::OpenOptionsExt, io::AsRawHandle}, sync::Arc};
use windows_sys::Win32::Storage::FileSystem::{
    SetFileInformationByHandle, FileDispositionInfo, FILE_DISPOSITION_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
};
use super::{validate_directory, validate_name, DirectoryHandle, PinnedDirectory};

// DELETE is a standard access right, independent of a provider's read broker.
const DELETE_ACCESS: u32 = 0x0001_0000;

impl DirectoryHandle {
    pub(crate) fn open_child_for_delete(&self, name: &OsStr) -> io::Result<Self> {
        validate_name(name)?;
        let path = self.0.path.join(name);
        let file = delete_open(&path, true)?;
        validate_directory(&file)?;
        Ok(Self(Arc::new(PinnedDirectory {
            file, path, _parent: Some(self.0.clone()), _ancestors: Vec::new(),
            consented_path: None,
        })))
    }

    /// Opening a final reparse entry with OPEN_REPARSE_POINT means deletion
    /// removes that entry. Ordinary directories require their original pin.
    pub(crate) fn remove_child(&self, name: &OsStr) -> io::Result<()> {
        validate_name(name)?;
        let file = delete_open(&self.0.path.join(name), false)?;
        let class = super::super::directory::classify_open_file(&file)?;
        if file.metadata()?.is_dir() && !class.link_like { return Err(changed()); }
        dispose(&file)
    }

    pub(crate) fn remove_empty_child(&self, name: &OsStr, expected: Self) -> io::Result<()> {
        validate_name(name)?;
        if expected.0.path != self.0.path.join(name)
            || !expected.0._parent.as_ref().is_some_and(|parent| Arc::ptr_eq(parent, &self.0))
        { return Err(changed()); }
        validate_directory(&expected.0.file)?;
        // expected is consumed. The directory is deleted when this last pin
        // closes; a nonempty directory fails here and remains retryable.
        dispose(&expected.0.file)
    }
}

fn delete_open(path: &std::path::Path, directory: bool) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .access_mode(DELETE_ACCESS | FILE_READ_ATTRIBUTES | if directory { FILE_LIST_DIRECTORY } else { 0 })
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}
fn dispose(file: &File) -> io::Result<()> {
    let mut info = FILE_DISPOSITION_INFO { DeleteFile: 1 };
    // SAFETY: the opened DELETE handle and correctly sized disposition buffer
    // stay live for this call. No freely reopened child path is involved.
    if unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfo,
        (&mut info as *mut FILE_DISPOSITION_INFO).cast(), size_of::<FILE_DISPOSITION_INFO>() as u32) } == 0
    { Err(io::Error::last_os_error()) } else { Ok(()) }
}
fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "opened delete entry changed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_host_windows_delete_consumes_original_delete_pin() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::create_dir(fixture.path().join("child")).unwrap();
        let parent = DirectoryHandle::open_root(fixture.path()).unwrap();
        let child = parent.open_child_for_delete(OsStr::new("child")).unwrap();
        assert!(std::fs::rename(fixture.path().join("child"), fixture.path().join("replacement")).is_err());
        parent.remove_empty_child(OsStr::new("child"), child).unwrap();
        assert!(!fixture.path().join("child").exists());
    }
}
