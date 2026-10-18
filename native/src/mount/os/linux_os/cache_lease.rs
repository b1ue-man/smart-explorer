//! Exclusive ownership of one drive cache under Linux and Android (DRV-M2),
//! the counterpart of the Windows lease: the same lock file
//! `<cache>/.<mount-id>.host.lock`, the same plain-directory checks, and
//! `flock(LOCK_EX | LOCK_NB)` in place of the zero Windows share mode. The
//! FUSE host (DRV-M4) and the documents provider (DRV-M8) hold it while an
//! engine owns the journal. `flock` binds to the open file description, so a
//! second lease fails even inside the same process.
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::Path;

use crate::mount::{MountId, MountRecovery};

/// Keeps the cache, mount and files directories open and the lock held
/// until dropped.
pub(crate) struct CacheLease {
    _lock_file: File,
    _files_directory: File,
    _mount_directory: File,
    _cache_directory: File,
}

impl CacheLease {
    pub(crate) fn acquire(cache_root: &Path, mount_id: &MountId) -> io::Result<Self> {
        let cache_directory = open_plain_directory(cache_root)?;
        let mount_path = cache_root.join(mount_id.as_str());
        let mount_directory = prepare_plain_directory(&mount_path)?;
        let files_directory = prepare_plain_directory(&mount_path.join("files"))?;
        let lock_path = cache_root.join(format!(".{}.host.lock", mount_id.as_str()));
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            // A final link fails with ELOOP instead of being followed.
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock_path)?;
        validate_lock_file(&lock_file)?;
        if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "the mounted-drive cache is in use by another drive host",
                ));
            }
            return Err(error);
        }
        Ok(Self {
            _lock_file: lock_file,
            _files_directory: files_directory,
            _mount_directory: mount_directory,
            _cache_directory: cache_directory,
        })
    }
}

/// Recovery state of a drive whose host is not running, read under the
/// lease so no host can write the journal meanwhile.
pub(crate) fn audit_recovery(cache_root: &Path, mount_id: &MountId) -> io::Result<MountRecovery> {
    match fs::symlink_metadata(cache_root.join(mount_id.as_str())) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(MountRecovery::Clean),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let _lease = CacheLease::acquire(cache_root, mount_id)?;
    crate::mount::spool::audit_recovery(cache_root, mount_id)
}

/// Journal records a drive whose host is not running still owns, read
/// under the lease (DRV-M7: the count of `RequiresUnlock`).
pub(crate) fn audit_pending(cache_root: &Path, mount_id: &MountId) -> io::Result<usize> {
    match fs::symlink_metadata(cache_root.join(mount_id.as_str())) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let _lease = CacheLease::acquire(cache_root, mount_id)?;
    crate::mount::spool::audit_pending(cache_root, mount_id)
}

fn prepare_plain_directory(path: &Path) -> io::Result<File> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    open_plain_directory(path)
}

fn open_plain_directory(path: &Path) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(unsafe_cache_object(
            "mount cache path is not a plain directory",
        ));
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?;
    // The checked path must still name the opened directory.
    let opened = directory.metadata()?;
    if !opened.is_dir() || opened.ino() != metadata.ino() || opened.dev() != metadata.dev() {
        return Err(unsafe_cache_object(
            "mount cache directory changed while it was opened",
        ));
    }
    Ok(directory)
}

fn validate_lock_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(unsafe_cache_object(
            "mount cache lease must be a single-link plain file of this user",
        ));
    }
    Ok(())
}

fn unsafe_cache_object(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
#[path = "cache_lease_tests.rs"]
mod tests;
