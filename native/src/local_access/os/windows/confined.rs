//! Pinned physical ancestry plus literal NT child operations for vault storage.
use super::{directory_handle::DirectoryHandle, regular};
use crate::local_access::LocalEntry;
use std::os::windows::io::AsRawHandle;
use std::{
    ffi::OsStr,
    fs::File,
    io,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
};
use windows_sys::Win32::Storage::FileSystem::{
    FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
    BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_WRITE_ATTRIBUTES,
};
#[path = "confined_names.rs"]
mod names;
#[path = "confined_native.rs"]
mod native;
#[path = "publication.rs"]
mod publication;

struct Pinned {
    file: File,
    parent: Option<Arc<Pinned>>,
    _root: Option<DirectoryHandle>,
    enumeration: Mutex<()>,
}
impl Drop for Pinned {
    fn drop(&mut self) {
        let mut parent = self.parent.take();
        while let Some(held) = parent {
            match Arc::try_unwrap(held) {
                Ok(mut held) => parent = held.parent.take(),
                Err(_) => break,
            }
        }
    }
}
#[derive(Clone)]
pub(crate) struct ConfinedDirectory {
    held: Arc<Pinned>,
}
impl ConfinedDirectory {
    pub(crate) fn open_root(path: &Path) -> io::Result<Self> {
        let root = DirectoryHandle::open_root(path)?;
        Ok(Self {
            held: Arc::new(Pinned {
                file: root.file().try_clone()?,
                parent: None,
                _root: Some(root),
                enumeration: Mutex::new(()),
            }),
        })
    }
    pub(crate) fn file(&self) -> &File {
        &self.held.file
    }
    pub(crate) fn open_publication_file(&self, name: &OsStr) -> io::Result<File> {
        publication::open_file(self.file(), name)
    }
    pub(crate) fn open_move_publication_file(&self, name: &OsStr) -> io::Result<File> {
        publication::open_move_file(self.file(), name)
    }
    pub(crate) fn confirm_publication_namespace(&self) -> io::Result<()> {
        publication::confirm_namespace(self.file())
    }
    pub(crate) fn child(&self, name: &OsStr) -> io::Result<Self> {
        let file = native::open(
            self.file(),
            name,
            0x1 | FILE_READ_ATTRIBUTES,
            1,
            0x1,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
        )?;
        let class = super::directory::classify_open_file(&file)?;
        if !file.metadata()?.is_dir() || class.link_like || class.special {
            return Err(boundary());
        }
        Ok(Self {
            held: Arc::new(Pinned {
                file,
                parent: Some(self.held.clone()),
                _root: None,
                enumeration: Mutex::new(()),
            }),
        })
    }
    pub(crate) fn list(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<LocalEntry>>>> {
        // Cloned handles share the kernel directory cursor. Serialize one
        // complete names snapshot, then release it before child metadata I/O.
        let _guard = self
            .held
            .enumeration
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(Box::new(names::read(self.file())?.into_iter().map(Ok)))
    }
    pub(crate) fn selected_entry(&self, name: &OsStr) -> io::Result<File> {
        self.entry(name)
    }
    pub(crate) fn select_child_root(&self, name: &OsStr) -> io::Result<Self> {
        self.child(name)
    }
    pub(crate) fn entry(&self, name: &OsStr) -> io::Result<File> {
        native::open(
            self.file(),
            name,
            FILE_READ_ATTRIBUTES,
            1,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
    }
    pub(crate) fn open_file(
        &self,
        name: &OsStr,
        writable: bool,
        exclusive: bool,
        truncate: bool,
        create: bool,
    ) -> io::Result<File> {
        let access = if writable {
            FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES
        } else {
            FILE_GENERIC_READ
        };
        let disposition = if exclusive {
            2
        } else if create {
            3
        } else {
            1
        };
        let options = 0x40 | if writable { 0x2 } else { 0 }; // NON_DIRECTORY, WRITE_THROUGH
        let opened = native::open(
            self.file(),
            name,
            access,
            disposition,
            options,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        );
        let file = match opened {
            Err(error)
                if writable
                    && !create
                    && !truncate
                    && error.kind() == io::ErrorKind::PermissionDenied =>
            {
                let guard = native::open(
                    self.file(),
                    name,
                    FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
                    1,
                    0x2,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                )?;
                regular::validate_file(&guard, true)?;
                let original = guard.metadata()?.permissions();
                if !original.readonly() {
                    return Err(error);
                }
                let mut writable = original.clone();
                writable.set_readonly(false);
                guard.set_permissions(writable)?;
                let opened = native::open(
                    self.file(),
                    name,
                    access,
                    1,
                    options,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                );
                guard.set_permissions(original)?;
                let opened = opened?;
                if identity(&guard)? != identity(&opened)? {
                    return Err(boundary());
                }
                opened
            }
            result => result?,
        };
        regular::validate_file(&file, true)?;
        if writable && information(&file)?.nNumberOfLinks != 1 {
            return Err(boundary());
        }
        if truncate {
            file.set_len(0)?;
        }
        Ok(file)
    }
    pub(crate) fn create_dir(&self, name: &OsStr) -> io::Result<Self> {
        let created = native::open(
            self.file(),
            name,
            FILE_READ_ATTRIBUTES,
            2,
            1 | 0x2,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
        )?;
        // Keep the exclusive created object pinned until the child handle is held.
        let directory = self.child(name)?;
        if identity(&created)? != directory.identity()? {
            return Err(boundary());
        }
        Ok(directory)
    }
    pub(crate) fn remove(&self, name: &OsStr, directory: bool) -> io::Result<()> {
        let file = native::open(
            self.file(),
            name,
            0x0001_0000 | FILE_READ_ATTRIBUTES,
            1,
            0x2,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
        )?;
        let class = super::directory::classify_open_file(&file)?;
        let is_dir = file.metadata()?.is_dir() && !class.link_like;
        if is_dir != directory {
            return Err(boundary());
        }
        let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&mut disposition as *mut FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    /// Transfer recovery binds validation and mutation to this same DELETE
    /// handle. Its sharing reservation prevents a final-child substitution.
    pub(crate) fn recover_expected_file(
        &self,
        name: &OsStr,
        expected: &File,
        destination: Option<&OsStr>,
    ) -> io::Result<()> {
        regular::validate_file(expected, true)?;
        let readonly = expected.metadata()?.permissions().readonly();
        let guard = native::open(
            self.file(),
            name,
            0x0001_0000
                | FILE_READ_ATTRIBUTES
                | if destination.is_none() && readonly {
                    FILE_WRITE_ATTRIBUTES
                } else {
                    0
                },
            1,
            0x2,
            FILE_SHARE_READ,
        )?;
        regular::validate_file(&guard, true)?;
        if identity(&guard)? != identity(expected)? {
            return Err(boundary());
        }
        if let Some(destination) = destination {
            native::rename(guard.try_clone()?, self.file(), destination, false)?;
            let reached = self.entry(destination)?;
            if identity(&reached)? != identity(&guard)? {
                return Err(boundary());
            }
            return Ok(());
        }
        use windows_sys::Win32::Storage::FileSystem::{
            FileDispositionInfoEx, FILE_DISPOSITION_INFO_EX,
        };
        // DELETE | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE. The latter
        // prevents a retained hardlink inheriting the target's read-only bit
        // from making its own quarantine undeletable after publication.
        let mut disposition = FILE_DISPOSITION_INFO_EX {
            Flags: 0x1 | 0x2 | 0x10,
        };
        if unsafe {
            SetFileInformationByHandle(
                guard.as_raw_handle(),
                FileDispositionInfoEx,
                (&mut disposition as *mut FILE_DISPOSITION_INFO_EX).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        } != 0
        {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124)) {
            return Err(error);
        }
        let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
        if unsafe {
            SetFileInformationByHandle(
                guard.as_raw_handle(),
                FileDispositionInfo,
                (&mut disposition as *mut FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(crate) fn rename(
        &self,
        name: &OsStr,
        target: &Self,
        target_name: &OsStr,
        replace: bool,
    ) -> io::Result<()> {
        let file = native::open(
            self.file(),
            name,
            0x0001_0000 | FILE_READ_ATTRIBUTES,
            1,
            0x2,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
        )?;
        let mut preserve_read_only = if replace {
            match target.entry(target_name) {
                Ok(existing) => {
                    existing.metadata()?.is_file() && existing.metadata()?.permissions().readonly()
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            }
        } else {
            false
        };
        let retained = file.try_clone()?;
        match native::rename(file, target.file(), target_name, replace) {
            Err(error) if replace && matches!(error.raw_os_error(), Some(1 | 50 | 87)) => {
                // Older/local filesystems may lack RenameInformationEx. The
                // basic relative rename remains atomic; only the held old
                // destination's read-only bit is lifted and restored on failure.
                let guard = match native::open(
                    target.file(),
                    target_name,
                    FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
                    1,
                    0x2,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ) {
                    Ok(guard) => Some(guard),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error),
                };
                let original = guard
                    .as_ref()
                    .map(|guard| guard.metadata().map(|meta| meta.permissions()))
                    .transpose()?;
                if let (Some(guard), Some(permissions)) = (&guard, &original) {
                    if permissions.readonly() {
                        preserve_read_only = true;
                        let mut writable = permissions.clone();
                        writable.set_readonly(false);
                        guard.set_permissions(writable)?;
                    }
                }
                let renamed =
                    native::rename_basic(retained.try_clone()?, target.file(), target_name);
                if renamed.is_err() {
                    if let (Some(guard), Some(permissions)) = (&guard, original) {
                        guard.set_permissions(permissions)?;
                    }
                }
                renamed?;
            }
            result => result?,
        }
        if preserve_read_only {
            let published = native::open(
                target.file(),
                target_name,
                FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
                1,
                0x2,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            )?;
            if identity(&published)? != identity(&retained)? {
                return Err(boundary());
            }
            let mut permissions = published.metadata()?.permissions();
            permissions.set_readonly(true);
            published.set_permissions(permissions)?;
        }
        Ok(())
    }
    /// FAT-style directory numbers need not survive rename/reopen; do not
    /// publish them as recovery ownership evidence for a movable child.
    pub(crate) fn stable_identity(&self) -> io::Result<Option<String>> {
        let mut name = [0u16; 261];
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW(
                self.file().as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                name.as_mut_ptr(),
                name.len() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let length = name
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(name.len());
        let name = String::from_utf16_lossy(&name[..length]);
        if !name.eq_ignore_ascii_case("NTFS") && !name.eq_ignore_ascii_case("ReFS") {
            return Ok(None);
        }
        self.identity().map(Some)
    }
    pub(crate) fn identity(&self) -> io::Result<String> {
        identity(self.file())
    }
}
fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}
fn identity(file: &File) -> io::Result<String> {
    super::directory_handle::identity::identity(file).map(|identity| identity.key())
}
fn boundary() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "local root boundary or regular-file identity changed",
    )
}
