//! Writing the digest cache (`hash_cache.rs`): what a write carries is built
//! under the cache's lock, the write itself (append or rewrite,
//! `hash_cache_file.rs`) runs outside it, and its outcome is taken back in
//! under the lock.
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::file::{self, NO_GENERATION};
use super::{
    lock, records, Key, CONFIRMED_DENIALS, FLUSH_INTERVAL, MAX_BLOCKS, MAX_RESIDENT, WRITE_WAIT,
};

/// The file contents of the cache and what they stand for, built under the
/// lock; the write itself runs outside it.
pub(super) struct Pending {
    pub(super) folder: PathBuf,
    /// The generation the file must still name when this write lands.
    pub(super) expected: u128,
    /// The generation written into `bytes`.
    pub(super) written: u128,
    /// The whole file, or (with `appended`) the blocks to append to it.
    pub(super) bytes: Vec<u8>,
    /// For an append: its number of blocks and records.
    pub(super) appended: Option<(usize, usize)>,
    /// For an append: the digests it carries (owed again if it fails).
    pub(super) taken: Vec<Key>,
}

pub(super) fn take_pending(force: bool) -> Option<Pending> {
    let deadline = Instant::now() + WRITE_WAIT;
    loop {
        let mut guard = lock();
        let cache = guard.as_mut()?;
        let folder = cache.folder.clone().filter(|_| cache.owes_write())?;
        if cache.writing {
            // Another thread writes: its file may lack the newest digests,
            // which stay owed. A forced flush waits for it.
            if !force || Instant::now() >= deadline {
                return None;
            }
            drop(guard);
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        if !force
            && cache
                .last_write
                .is_some_and(|written| written.elapsed() < FLUSH_INTERVAL)
        {
            return None;
        }
        cache.writing = true;
        let expected = cache.generation;
        if expected == NO_GENERATION {
            cache.generation = file::new_generation();
        }
        let written = cache.generation;
        let mut pending = Pending {
            folder,
            expected,
            written,
            bytes: Vec::new(),
            appended: None,
            taken: Vec::new(),
        };
        if cache.rewrite || expected == NO_GENERATION {
            cache.rewrite = false;
            cache.unwritten.clear();
            pending.bytes = records::whole_file(cache);
        } else {
            pending.taken = cache.unwritten.drain().collect();
            let (bytes, blocks, count) = records::appended(cache, &pending.taken);
            pending.bytes = bytes;
            pending.appended = Some((blocks, count));
        }
        return Some(pending);
    }
}

/// The whole file from memory, for an append the file has no room for. The
/// digests owed so far are in it.
fn rewrite_now(pending: &Pending) -> io::Result<Vec<u8>> {
    let mut guard = lock();
    match guard.as_mut() {
        Some(cache)
            if cache.folder.as_deref() == Some(pending.folder.as_path())
                && cache.generation == pending.written =>
        {
            cache.unwritten.clear();
            Ok(records::whole_file(cache))
        }
        _ => Err(io::Error::other("the digest cache was set up again")),
    }
}

/// Appends the pending digests, or replaces the file when there is no file
/// of this generation yet or it has no room.
fn perform(pending: &mut Pending) -> io::Result<file::Replaced> {
    #[cfg(test)]
    if let Some(kind) = super::forced_write_error() {
        return Err(io::Error::from(kind));
    }
    if let Some((blocks, count)) = pending.appended {
        let found = file::append_if_generation(
            &pending.folder,
            pending.expected,
            (&pending.bytes, blocks, count),
            (MAX_BLOCKS, MAX_RESIDENT),
        )?;
        match found {
            file::Appended::Written => return Ok(file::Replaced::Written),
            file::Appended::Stale => return Ok(file::Replaced::Stale),
            file::Appended::Compact => {
                pending.bytes = rewrite_now(pending)?;
                pending.appended = None;
                pending.taken.clear();
            }
        }
    }
    file::replace_if_generation(&pending.folder, pending.expected, &pending.bytes)
}

pub(super) fn write_pending(force: bool) {
    let Some(mut pending) = take_pending(force) else {
        return;
    };
    let outcome = perform(&mut pending);
    settle(pending, outcome);
}

/// Takes the outcome of a write into the cache.
fn settle(pending: Pending, outcome: io::Result<file::Replaced>) {
    let mut guard = lock();
    let Some(cache) = guard.as_mut() else {
        return;
    };
    cache.writing = false;
    if cache.folder.as_deref() != Some(pending.folder.as_path()) {
        return;
    }
    match outcome {
        Ok(file::Replaced::Written) => {
            cache.last_write = Some(Instant::now());
            cache.denied = 0;
        }
        Ok(file::Replaced::Stale) => {
            // Another process cleared the file since this one loaded it: the
            // digests held here are forgotten, and the file is read again on
            // the next use.
            cache.forget();
            cache.generation = NO_GENERATION;
            cache.loaded = false;
        }
        Err(error) => {
            // The file keeps its generation; the digests are owed again.
            if cache.generation == pending.written {
                cache.generation = pending.expected;
            }
            if pending.appended.is_some() {
                for key in pending.taken {
                    if cache.entries.contains_key(&key) {
                        cache.unwritten.insert(key);
                    }
                }
            } else {
                cache.rewrite = true;
            }
            if error.kind() == io::ErrorKind::PermissionDenied {
                cache.denied += 1;
            } else {
                cache.denied = 0;
            }
            if cache.denied >= CONFIRMED_DENIALS {
                cache.write_off = true;
                cache.rewrite = false;
                cache.unwritten.clear();
            }
        }
    }
}
