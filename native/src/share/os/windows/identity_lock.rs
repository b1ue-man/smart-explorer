use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

const LOCK_DIRECTORY: &str = "identity-lock-v1";
const LOCK_FILE: &str = "transaction.lock";

pub(super) struct IdentityLock {
    _file: File,
}

pub(super) fn acquire(app_data_dir: &Path) -> io::Result<IdentityLock> {
    let directory = app_data_dir.join(LOCK_DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(LOCK_FILE);
    loop {
        match open_exclusive(&path) {
            Ok(file) => {
                if !file.metadata()?.file_type().is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Share identity lock must be a regular file, not a reparse point",
                    ));
                }
                return Ok(IdentityLock { _file: file });
            }
            Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // A zero share mode is system-wide, including other Windows sessions.
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}
