//! Device-wide exclusion of one sync pair (V3, Y18/Y45/Y79): a run, a
//! conflict resolution or a version restore holds the pair's lock, so the
//! desktop window, the background service and the Android facade never work
//! on the same pair at once. The lock is a file lock in the sync data
//! directory; the operating system releases it when its holder ends, also
//! after a crash.
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::vfs::Backend;

/// How often a waiting caller tries again.
const RETRY_SLICE: Duration = Duration::from_millis(200);
/// Longest accepted lock identity (hex digits).
const MAX_ID_LEN: usize = 64;

/// Held while one run, conflict resolution or restore of a pair is in
/// progress; released when dropped.
#[derive(Debug)]
pub struct PairLock {
    id: String,
    _file: File,
}

impl PairLock {
    /// Takes the pair's lock without waiting. `ErrorKind::WouldBlock` when
    /// another run, resolution or restore of the pair holds it, in this or in
    /// another process ("läuft bereits").
    pub fn acquire(lock_id: &str) -> io::Result<PairLock> {
        let id = lock_id.to_ascii_lowercase();
        let path = lock_path(&id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(PairLock { id, _file: file }),
            Err(TryLockError::WouldBlock) => Err(busy()),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// Waits up to `wait` for the lock: `ErrorKind::WouldBlock` when it stays
    /// taken, `ErrorKind::Interrupted` once `cancel` is set.
    pub fn acquire_wait(
        lock_id: &str,
        wait: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<PairLock> {
        let deadline = Instant::now() + wait;
        loop {
            match Self::acquire(lock_id) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                result => return result,
            }
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Warten auf die Synchronisierungssperre abgebrochen",
                ));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(busy());
            }
            std::thread::sleep(RETRY_SLICE.min(deadline - now));
        }
    }

    /// The lock identity (`pair_lock_id`, lower case).
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Unordered identity of a pair for its lock: A↔B and B↔A exclude each other,
/// since two jobs may list the same folders the other way round.
pub fn pair_lock_id(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str) -> String {
    let mut parts = [(a.state_identity(), root_a), (b.state_identity(), root_b)];
    parts.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    feed(&mut hash, b"smart-explorer/bisync-pair-lock/v1");
    for (identity, root) in &parts {
        feed(&mut hash, &(identity.len() as u64).to_be_bytes());
        feed(&mut hash, identity.as_bytes());
        feed(&mut hash, &(root.len() as u64).to_be_bytes());
        feed(&mut hash, root.as_bytes());
    }
    format!("{hash:016x}")
}

fn feed(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

fn lock_path(lock_id: &str) -> io::Result<PathBuf> {
    if lock_id.is_empty()
        || lock_id.len() > MAX_ID_LEN
        || !lock_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid sync pair lock id",
        ));
    }
    Ok(crate::support_dirs::sync_data_dir()
        .join("locks")
        .join(format!("pair-{lock_id}.lock")))
}

fn busy() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "Eine Synchronisierung oder Konfliktauflösung dieses Paars läuft bereits.",
    )
}

#[cfg(test)]
#[path = "pair_lock_tests.rs"]
mod tests;
