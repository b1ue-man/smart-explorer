//! Exclusive per-job lock of the job state on Windows (RV1, V4): `LockFileEx`
//! on `job-state/<id>.lock`. Byte-range locks belong to the handle, so they
//! exclude other processes and other threads of this process alike (each
//! writer opens its own handle); the guard unlocks before the handle closes.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, UnlockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

const RETRY: Duration = Duration::from_millis(20);

/// Held while a writer changes the state; dropping it unlocks and closes the
/// handle.
pub(super) struct StateLock {
    file: File,
}

impl StateLock {
    /// Waits at most `wait` for the lock; `TimedOut` when another writer keeps
    /// it longer.
    pub(super) fn acquire(path: &Path, wait: Duration) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let deadline = Instant::now() + wait;
        loop {
            // SAFETY: an all-zero OVERLAPPED is valid (offset 0, no event).
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            // SAFETY: the handle is open for the duration of the call; the
            // whole 64-bit range is locked, which is allowed beyond the end.
            let locked = unsafe {
                LockFileEx(
                    raw_handle(&file),
                    LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                    0,
                    u32::MAX,
                    u32::MAX,
                    &mut overlapped,
                )
            };
            if locked != 0 {
                return Ok(Self { file });
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_LOCK_VIOLATION as i32) {
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

impl Drop for StateLock {
    fn drop(&mut self) {
        // SAFETY: as above; closing the handle would release the range as
        // well, but only when the system gets to it.
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        unsafe {
            UnlockFileEx(
                raw_handle(&self.file),
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            );
        }
    }
}

fn raw_handle(file: &File) -> HANDLE {
    file.as_raw_handle()
}
