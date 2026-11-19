//! Read-modify-write of `viewer.json` under a lock every process shares
//! (Codex 8/12): the viewer, a second viewer and `se screen view` write the
//! same file, and without it the later of two writers took the other's
//! field back. The write itself stays atomic (`write_private_atomic`: a
//! private temporary file renamed over the old one).
use std::fs::{OpenOptions, TryLockError};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use super::ViewerPrefs;

/// How long a writer waits for another one (each holds the lock for one
/// read and one small write).
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// Reads the file at `path`, lets `change` alter it and writes it back,
/// all under `<path>.lock`; an error when the lock cannot be had.
pub fn update(path: &Path, change: impl FnOnce(&mut ViewerPrefs)) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the viewer settings need a folder",
        )
    })?;
    crate::support_dirs::ensure_private_dir(parent)?;
    let lock_path = path.with_extension("json.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "{}: another process keeps the viewer settings locked",
                        lock_path.display()
                    ),
                ))
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
    let mut prefs = ViewerPrefs::load_from(path);
    change(&mut prefs);
    // Closing `lock` after the write releases it.
    prefs.save_to(path)
}
