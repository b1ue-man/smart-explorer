use std::fs::File;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

const LOCK_DIRECTORY: &str = "identity-lock-v1";
const LOCK_FILE: &str = "transaction.lock";

pub(super) struct IdentityLock {
    _file: File,
}

pub(super) fn acquire(app_data_dir: &Path) -> io::Result<IdentityLock> {
    acquire_until(app_data_dir, Instant::now() + Duration::from_secs(5))
}

pub(super) fn acquire_until(app_data_dir: &Path, deadline: Instant) -> io::Result<IdentityLock> {
    let directory = app_data_dir.join(LOCK_DIRECTORY);
    crate::support_dirs::ensure_private_dir(&directory)?;
    let path = directory.join(LOCK_FILE);
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut,
                "Share identity is busy; retry after the other operation finishes"));
        }
        match open_exclusive(&path) {
            Ok(file) => {
                match file.try_lock() {
                    Ok(()) => return Ok(IdentityLock { _file: file }),
                    Err(std::fs::TryLockError::WouldBlock) => {},
                    Err(std::fs::TryLockError::Error(error)) => return Err(error),
                }
            }
            Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => {
            }
            Err(error) => return Err(error),
        }
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(25)));
    }
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    crate::support_dirs::open_private_lock(path)
}
