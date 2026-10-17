//! Single instance of the se Desktop shell on Linux: `flock` on
//! `<AD>/desktop/shell.lock` (C5.5), the daemon's pattern with an own file.
//! The kernel releases the lock when the shell ends, also after a crash.
use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use super::super::store::DesktopDir;

/// The shell's lock; released when dropped.
pub(crate) struct ShellInstance {
    _file: File,
}

/// Takes the lock, waiting up to `wait` (a status probe holds a shared lock
/// for an instant). `Ok(None)`: another shell runs.
pub(crate) fn acquire(dir: &DesktopDir, wait: Duration) -> Result<Option<ShellInstance>, String> {
    let path = dir.instance_lock_path();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(ShellInstance { _file: file })),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => return Err(format!("{}: {error}", path.display())),
        }
    }
}

/// Whether a shell holds the lock. Never creates the file or the folder.
pub(crate) fn is_running(dir: &DesktopDir) -> bool {
    let Ok(file) = OpenOptions::new().read(true).open(dir.instance_lock_path()) else {
        return false;
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => false,
    }
}
