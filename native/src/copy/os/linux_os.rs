use std::fs::{File, Metadata};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub(super) type FileIdentity = (u64, u64);

pub(super) fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    let left = std::fs::File::open(left)?;
    let right = std::fs::File::open(right)?;
    Ok(file_identity(&left)? == file_identity(&right)?)
}

pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

pub(super) fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(file_identity(&file)? == expected)
}

pub(super) fn metadata_is_link_like(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(super) fn path_text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not valid Unicode"))
}

pub(super) fn path_key(path: &Path) -> io::Result<String> {
    path_text(path)
}

pub(super) fn commit_staged(staged: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        return std::fs::rename(staged, dest);
    }
    rename_no_replace(staged, dest)
}

pub(super) fn move_file(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        return std::fs::rename(src, dest);
    }
    rename_no_replace(src, dest)
}

pub(super) fn is_cross_device(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(18)
}

pub(super) fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "source path contains a NUL byte",
        )
    })?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination path contains a NUL byte",
        )
    })?;
    // The libc crate omits the renameat2 wrapper on musl. The raw Linux
    // syscall preserves the required atomic no-replace contract on both libc
    // families; ENOSYS must propagate instead of falling back to a racy probe.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
