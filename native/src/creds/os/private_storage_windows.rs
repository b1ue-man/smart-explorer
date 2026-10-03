//! Windows private objects reuse the V1 protected-DACL implementation.
use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) fn ensure_directory(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            use std::os::windows::fs::MetadataExt;
            if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "private directory must not be a reparse point",
                ));
            }
            crate::local_access::DirectoryHandle::open_root(path)?.secure_private()
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "private directory needs a parent",
                    )
                })?;
            if !parent.exists() {
                ensure_directory(parent)?;
            }
            let name = path.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private directory needs a name",
                )
            })?;
            let root = crate::local_access::DirectoryHandle::open_root(parent)?;
            match root.create_private_child(name) {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    ensure_directory(path)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn create_file(path: &Path) -> io::Result<File> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "private file needs a parent")
    })?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "private file needs a name"))?;
    ensure_directory(parent)?;
    let root = crate::local_access::DirectoryHandle::open_root(parent)?;
    root.secure_private()?;
    root.create_file_new(name)
}

pub(crate) fn open_file(path: &Path, writable: bool) -> io::Result<File> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("private file needs a parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("private file needs a name"))?;
    let root = crate::local_access::DirectoryHandle::open_root(parent)?;
    root.open_private_child(name, writable)
}

pub(crate) fn secure(file: &File, directory: bool) -> io::Result<()> {
    crate::local_access::secure_private_handle(file, directory)
}

pub(crate) fn sync_directory(_path: &Path) -> io::Result<()> {
    // Win32 has no documented durable directory-flush contract. The private
    // writer flushes the file and uses V1's write-through promotion instead.
    Ok(())
}
