//! The digests one run takes, so no file is read twice in that run when two
//! walks of the same local tree ask on different threads (`LocalReads::run`).
//! Whichever asks first for a file reads it; the other waits for that read
//! and takes its result (single flight). Each digest is handed out twice
//! (once per walk) and then forgotten; one a walk does not ask for again
//! stays until the run ends. A local sync side no longer needs it: its key
//! phase and its snapshot are one walk (`bisync::snapshot_local_keys`), which
//! asks without run digests.
//!
//! A digest here is a digest of this run: the file was read after the run
//! began, so it also answers the checksum mode's demand for a fresh read.
//! The key names the file by path, size and modification time, so a file
//! that changed between the two asks is read again.
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How often a waiting thread looks at the stop flag.
const WAIT_SLICE: Duration = Duration::from_millis(50);
/// Asks that take a digest out of the table (the two walks of a side).
const TAKERS: u8 = 2;

/// Path, size and modification time (ns) of the file read.
pub type FileKey = (PathBuf, u64, i128);

enum Slot {
    /// One thread reads the file.
    Reading,
    Ready {
        digest: String,
        taken: u8,
    },
}

/// The digests of one run (shared by the walks of one local side).
#[derive(Default)]
pub struct RunDigests {
    slots: Mutex<HashMap<FileKey, Slot>>,
    changed: Condvar,
}

/// Removes the claim of a reader that did not finish (an error or a panic),
/// so waiting threads read the file themselves.
struct Claim<'a> {
    run: &'a RunDigests,
    key: Option<FileKey>,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.run.lock().remove(&key);
            self.run.changed.notify_all();
        }
    }
}

impl RunDigests {
    fn lock(&self) -> MutexGuard<'_, HashMap<FileKey, Slot>> {
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The digest of the file `key` names in this run: read by `read` unless
    /// another thread reads it or has read it.
    pub fn once(
        &self,
        key: FileKey,
        stop: &AtomicBool,
        read: impl FnOnce() -> io::Result<String>,
    ) -> io::Result<String> {
        let mut slots = self.lock();
        loop {
            match slots.get_mut(&key) {
                None => {
                    slots.insert(key.clone(), Slot::Reading);
                    break;
                }
                Some(Slot::Reading) => {
                    if stop.load(Ordering::Relaxed) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "digest wait canceled",
                        ));
                    }
                    slots = self
                        .changed
                        .wait_timeout(slots, WAIT_SLICE)
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .0;
                }
                Some(Slot::Ready { digest, taken }) => {
                    let digest = digest.clone();
                    *taken += 1;
                    if *taken >= TAKERS {
                        slots.remove(&key);
                    }
                    return Ok(digest);
                }
            }
        }
        drop(slots);
        let mut claim = Claim {
            run: self,
            key: Some(key.clone()),
        };
        let digest = read()?;
        claim.key = None;
        self.lock().insert(
            key,
            Slot::Ready {
                digest: digest.clone(),
                taken: 1,
            },
        );
        self.changed.notify_all();
        Ok(digest)
    }

    /// Digests still held (a file only one walk asked for).
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
#[path = "run_digests_tests.rs"]
mod tests;
