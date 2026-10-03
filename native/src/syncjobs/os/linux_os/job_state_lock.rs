//! Exclusive per-job lock of the job state on Linux and Android (RV1, V4):
//! `flock` on `job-state/<id>.lock`. The lock belongs to the open file
//! description, so it excludes other processes and other threads of this
//! process alike (each writer opens its own descriptor) and ends with the
//! descriptor, also when a writer crashes.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

const RETRY: Duration = Duration::from_millis(20);

/// Held while a writer changes the state; dropping it closes the descriptor
/// and releases the lock.
pub(crate) struct StateLock {
    _file: File,
}

impl StateLock {
    /// Waits at most `wait` for the lock; `TimedOut` when another writer keeps
    /// it longer.
    pub(crate) fn acquire(path: &Path, wait: Duration) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        let deadline = Instant::now() + wait;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let busy = error
                .raw_os_error()
                .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN);
            if !busy {
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "job state is locked by another writer",
                ));
            }
            std::thread::sleep(RETRY);
        }
    }
}
