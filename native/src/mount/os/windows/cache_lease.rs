use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};

use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use crate::mount::MountId;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Keeps one mount journal under the exclusive ownership of this host process.
///
/// The zero Windows share mode prevents a second process from opening, replacing,
/// renaming, or deleting the lease object until this value is dropped.
pub(super) struct CacheLease {
    _lock_file: File,
    _files_directory: File,
    _mount_directory: File,
    _cache_directory: File,
}

impl CacheLease {
    pub(super) fn acquire(cache_root: &Path, mount_id: &MountId) -> io::Result<Self> {
        let cache_directory = open_plain_directory(cache_root)?;
        let mount_path = cache_root.join(mount_id.as_str());
        let mount_directory = prepare_plain_directory(&mount_path)?;
        let files_directory = prepare_plain_directory(&mount_path.join("files"))?;
        let lock_path = cache_root.join(format!(".{}.host.lock", mount_id.as_str()));
        validate_existing_target(&lock_path)?;
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            // A zero share mode is system-wide, including other Windows sessions.
            .share_mode(0)
            // Open a final reparse point itself so the handle check can reject it.
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(lock_path)?;
        validate_lock_handle(&lock_file)?;
        Ok(Self {
            _lock_file: lock_file,
            _files_directory: files_directory,
            _mount_directory: mount_directory,
            _cache_directory: cache_directory,
        })
    }
}

pub(crate) fn audit_recovery(
    cache_root: &Path,
    mount_id: &MountId,
) -> io::Result<crate::mount::MountRecovery> {
    match fs::symlink_metadata(cache_root.join(mount_id.as_str())) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(crate::mount::MountRecovery::Clean)
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let _lease = CacheLease::acquire(cache_root, mount_id)?;
    crate::mount::spool::audit_recovery(cache_root, mount_id)
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
    if !metadata.is_dir() || is_reparse_point(&metadata) {
        return Err(unsafe_cache_object(
            "mount cache path is not a reparse-free plain directory",
        ));
    }
    let directory = OpenOptions::new()
        .read(true)
        // Allow normal cache access while denying rename/delete of the directory.
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate_directory_handle(&directory)?;
    Ok(directory)
}

fn validate_existing_target(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || is_reparse_point(&metadata) => Err(
            unsafe_cache_object("mount cache lease target is not a plain file"),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_directory_handle(directory: &File) -> io::Result<()> {
    let information = file_information(directory)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(unsafe_cache_object(
            "mount cache directory handle is not a reparse-free directory",
        ));
    }
    Ok(())
}

fn validate_lock_handle(file: &File) -> io::Result<()> {
    let information = file_information(file)?;
    if information.nNumberOfLinks != 1
        || information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != 0
    {
        return Err(unsafe_cache_object(
            "mount cache lease must be a single-link plain file",
        ));
    }
    Ok(())
}

fn file_information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetFileInformationByHandle(file.as_raw_handle() as _, &mut information as *mut _)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(information)
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn unsafe_cache_object(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
