//! Persistent digest cache on the side that stores the files (E9): the SSH
//! agent, a Share host and a local sync side keep the digests they computed,
//! keyed by the file's status (`local_platform::file_stamp`: device and
//! inode, or volume serial and 128-bit file ID on Windows, size,
//! modification and status-change time in nanoseconds) and the algorithm. A
//! digest is used again only while that status is unchanged and not racy.
//!
//! Racy timestamps (Git, `Documentation/technical/racy-git.txt`): a file
//! changed within the time granularity of the moment its digest was taken
//! can change again with the same times. So a digest counts only when the
//! file's modification and change times lie more than `RACY_SLACK` before
//! the moment the digest was taken (`recorded_ns`); otherwise it is computed
//! again each time until the file is old enough.
//!
//! Only file systems with durable file IDs have a stamp (`file_ids.rs`: not
//! FAT, exFAT, FUSE, SMB/CIFS, 9p or optical media; on Windows only NTFS and
//! ReFS): a file without a stamp is never cached. A run that asks for a fresh
//! read (`digest_fresh`, the checksum compare mode of a local sync side)
//! never takes a stored digest, and stores the one it read.
//!
//! The file holds at most `MAX_ENTRIES` digests, the most recently used
//! first, and the generation of the file (`hash_cache_file.rs`). A cache that
//! cannot be read or written costs only the reuse, never a result. New
//! digests are collected in memory and written together (`flush`: at most
//! every `FLUSH_INTERVAL`; `flush_now`: at the end of a walk or a connection),
//! as one atomic replacement of the file that is built under the lock and
//! written outside it, so no thread waits for a write. The cache lives in the
//! app's data folder (`support_dirs::app_data_dir`, on Android the host's) or,
//! in the standalone agent, in its cache folder (`$XDG_CACHE_HOME/se-agent` or
//! `~/.cache/se-agent`, `se-agent/src/support_dirs.rs`).
//!
//! `clear` forgets every digest of this process and gives the file a new
//! generation, so a writer that still holds an older generation drops its
//! digests instead of writing them back (`write_pending`). A process the app
//! cannot reach (another command, a second instance) sees the new generation
//! within `GENERATION_CHECK` and forgets its digests then
//! (`forget_if_cleared`); it never writes them back. The app clears a running
//! background worker through its IPC at once (`daemon::clear_hash_cache`,
//! `se doctor --clear-hash-cache`).
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "hash_cache_file.rs"]
mod file;
use file::{FILE_NAME, MAGIC, NO_GENERATION};

/// Digests kept (about 100 bytes each in the file).
const MAX_ENTRIES: usize = 500_000;
/// Times closer than this to the moment of hashing are "racy" (two seconds
/// cover the coarsest common time granularity, FAT).
const RACY_SLACK_NS: i128 = 2_000_000_000;
/// New digests are written at most this often by `flush`.
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
/// How long `flush_now` waits for a write of another thread.
const WRITE_WAIT: Duration = Duration::from_secs(5);
/// How often a process looks whether another process cleared the file: a
/// clear reaches a process the app cannot ask (another command, a second
/// instance) within this time. Each look reads the file's 21-byte header.
const GENERATION_CHECK: Duration = Duration::from_secs(1);

struct Entry {
    stamp: [i128; 5],
    recorded_ns: i128,
    used: u64,
    digest: String,
}

#[derive(Default)]
struct Cache {
    folder: Option<PathBuf>,
    configured: bool,
    loaded: bool,
    /// Digests were added since the last write (`rev` counts the additions).
    dirty: bool,
    rev: u64,
    /// The generation of the file the digests belong to (`NO_GENERATION`: none).
    generation: u128,
    /// A thread is writing the file (outside the lock).
    writing: bool,
    /// When the file's generation was last compared with `generation`.
    generation_checked: Option<Instant>,
    last_write: Option<Instant>,
    clock: u64,
    entries: HashMap<(i128, i128, u8), Entry>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Cache>> {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as i128)
}

/// Keeps the cache file in `folder`; `None` keeps digests for this process
/// only.
pub(crate) fn configure(folder: Option<PathBuf>) {
    let mut guard = lock();
    let cache = guard.get_or_insert_with(Cache::default);
    if cache.configured && cache.folder == folder {
        return;
    }
    *cache = Cache {
        folder,
        configured: true,
        ..Cache::default()
    };
}

/// `configure`, unless a folder was configured already.
pub(crate) fn configure_once(folder: impl FnOnce() -> Option<PathBuf>) {
    let mut guard = lock();
    if guard.as_ref().is_some_and(|cache| cache.configured) {
        return;
    }
    *guard = Some(Cache {
        folder: folder(),
        configured: true,
        ..Cache::default()
    });
}

/// The folder of a cache nobody configured: the data folder of the process
/// (`crate::support_dirs::app_data_dir`: the app's on every platform, the
/// standalone agent's cache folder, empty when it has none). None in tests.
fn default_folder() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let folder = crate::support_dirs::app_data_dir();
    (!folder.as_os_str().is_empty()).then_some(folder)
}

fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut guard = lock();
    let cache = guard.get_or_insert_with(|| Cache {
        folder: default_folder(),
        configured: true,
        ..Cache::default()
    });
    if !cache.loaded {
        cache.loaded = true;
        cache.generation_checked = Some(Instant::now());
        if let Some(folder) = cache.folder.clone() {
            load(cache, &folder.join(FILE_NAME));
        }
    } else {
        forget_if_cleared(cache);
    }
    f(cache)
}

/// Forgets the digests of this process when another process cleared the
/// file since they were loaded (the file names another generation), and
/// reads the file again. Looks at most every `GENERATION_CHECK`, and not
/// while this process writes the file (its first write gives it a new
/// generation before the file has it).
fn forget_if_cleared(cache: &mut Cache) {
    let Some(folder) = cache.folder.clone() else {
        return;
    };
    let due = cache
        .generation_checked
        .is_none_or(|checked| checked.elapsed() >= GENERATION_CHECK);
    if !due || cache.writing {
        return;
    }
    cache.generation_checked = Some(Instant::now());
    if file::generation_on_disk(&folder) == cache.generation {
        return;
    }
    cache.entries.clear();
    cache.dirty = false;
    cache.rev += 1;
    load(cache, &folder.join(FILE_NAME));
}

/// The digest (`algorithm`: `ops_types::digest` code) of the regular file
/// whose current status is `stamp` (`local_platform::file_stamp` or
/// `path_stamp`): the stored one while the status is unchanged and not racy,
/// else `compute()`, stored when the file kept its status (`after` is read
/// once the digest was computed).
pub(crate) fn digest(
    stamp: Option<[i128; 5]>,
    algorithm: u8,
    compute: impl FnOnce() -> io::Result<String>,
    after: impl FnOnce() -> Option<[i128; 5]>,
) -> io::Result<String> {
    digest_with(true, stamp, algorithm, compute, after)
}

/// `digest` that never takes a stored digest: the file is read, and the
/// digest read is stored (the checksum compare mode asks for the content as
/// it is now).
pub(crate) fn digest_fresh(
    stamp: Option<[i128; 5]>,
    algorithm: u8,
    compute: impl FnOnce() -> io::Result<String>,
    after: impl FnOnce() -> Option<[i128; 5]>,
) -> io::Result<String> {
    digest_with(false, stamp, algorithm, compute, after)
}

fn digest_with(
    reuse: bool,
    stamp: Option<[i128; 5]>,
    algorithm: u8,
    compute: impl FnOnce() -> io::Result<String>,
    after: impl FnOnce() -> Option<[i128; 5]>,
) -> io::Result<String> {
    let Some(stamp) = stamp else {
        return compute();
    };
    let key = (stamp[0], stamp[1], algorithm);
    let hit = reuse
        .then(|| {
            with_cache(|cache| {
                cache.clock += 1;
                let clock = cache.clock;
                cache.entries.get_mut(&key).and_then(|entry| {
                    let fresh = entry.stamp == stamp
                        && stamp[3] < entry.recorded_ns - RACY_SLACK_NS
                        && stamp[4] < entry.recorded_ns - RACY_SLACK_NS;
                    fresh.then(|| {
                        entry.used = clock;
                        entry.digest.clone()
                    })
                })
            })
        })
        .flatten();
    if let Some(digest) = hit {
        return Ok(digest);
    }
    let recorded_ns = now_ns();
    let digest = compute()?;
    let unchanged = after().is_some_and(|after| after == stamp);
    if unchanged {
        with_cache(|cache| {
            cache.clock += 1;
            let used = cache.clock;
            cache.entries.insert(
                key,
                Entry {
                    stamp,
                    recorded_ns,
                    used,
                    digest: digest.clone(),
                },
            );
            cache.dirty = true;
            cache.rev += 1;
        });
    }
    Ok(digest)
}

/// Writes the digests added since the last write, unless the last write was
/// less than `FLUSH_INTERVAL` ago (a request that may repeat per file).
pub(crate) fn flush() {
    write_pending(false);
}

/// Writes the digests added since the last write now (the end of a walk, of
/// a connection or of a run).
pub(crate) fn flush_now() {
    write_pending(true);
}

/// Forgets every digest of this process and gives the cache file of `folder`
/// a new generation without digests (`file::clear_file`). A background worker
/// keeps its own memory: it is cleared through its IPC.
pub(crate) fn clear(folder: &Path) -> io::Result<()> {
    let generation = file::clear_file(folder)?;
    if let Some(cache) = lock().as_mut() {
        cache.entries.clear();
        cache.dirty = false;
        cache.rev += 1;
        if cache.folder.as_deref() == Some(folder) {
            cache.generation = generation;
            cache.loaded = true;
        } else {
            // Another folder's cache: its own file is read again when used.
            cache.generation = NO_GENERATION;
            cache.loaded = false;
        }
    }
    Ok(())
}

/// The file contents of the cache and what they stand for, built under the
/// lock; the write itself runs outside it.
struct Pending {
    folder: PathBuf,
    /// The generation the file must still name when this write lands.
    expected: u128,
    /// The generation written into `bytes`.
    written: u128,
    bytes: Vec<u8>,
    rev: u64,
}

fn take_pending(force: bool) -> Option<Pending> {
    let deadline = Instant::now() + WRITE_WAIT;
    loop {
        let mut guard = lock();
        let cache = guard.as_mut()?;
        let folder = cache.folder.clone().filter(|_| cache.dirty)?;
        if cache.writing {
            // Another thread writes: its file may lack the newest digests,
            // which stay dirty. A forced flush waits for it.
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
        if cache.entries.len() > MAX_ENTRIES {
            let mut used: Vec<u64> = cache.entries.values().map(|entry| entry.used).collect();
            used.sort_unstable_by(|left, right| right.cmp(left));
            let oldest_kept = used[MAX_ENTRIES - 1];
            cache.entries.retain(|_, entry| entry.used >= oldest_kept);
        }
        cache.writing = true;
        let expected = cache.generation;
        if expected == NO_GENERATION {
            cache.generation = file::new_generation();
        }
        return Some(Pending {
            folder,
            expected,
            written: cache.generation,
            bytes: encode(cache),
            rev: cache.rev,
        });
    }
}

fn write_pending(force: bool) {
    let Some(pending) = take_pending(force) else {
        return;
    };
    let outcome = file::replace_if_generation(&pending.folder, pending.expected, &pending.bytes);
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
            // Digests added meanwhile stay dirty for the next write.
            if cache.rev == pending.rev {
                cache.dirty = false;
            }
        }
        Ok(file::Replaced::Stale) => {
            // Another process cleared the file since this one loaded it: the
            // digests held here are forgotten, and the file is read again on
            // the next use.
            cache.entries.clear();
            cache.dirty = false;
            cache.generation = NO_GENERATION;
            cache.loaded = false;
        }
        Err(_) => {
            // The file keeps its generation; the digests stay dirty.
            if cache.generation == pending.written {
                cache.generation = pending.expected;
            }
        }
    }
}

fn load(cache: &mut Cache, path: &Path) {
    cache.generation = NO_GENERATION;
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Some(generation) = file::generation_in(&bytes) else {
        return;
    };
    let Some(mut input) = bytes.get(file::HEADER_LEN..) else {
        return;
    };
    cache.generation = generation;
    let mut read = |count: usize| -> Option<Vec<u8>> {
        if input.len() < count {
            return None;
        }
        let (head, rest) = input.split_at(count);
        input = rest;
        Some(head.to_vec())
    };
    let int =
        |bytes: Vec<u8>| -> Option<i128> { Some(i128::from_be_bytes(bytes.try_into().ok()?)) };
    while cache.entries.len() < MAX_ENTRIES {
        let Some(algorithm) = read(1) else {
            break;
        };
        let mut stamp = [0i128; 5];
        for slot in &mut stamp {
            match read(16).and_then(int) {
                Some(value) => *slot = value,
                None => return,
            }
        }
        let Some(recorded_ns) = read(16).and_then(int) else {
            return;
        };
        let Some(length) = read(1) else {
            return;
        };
        let Some(digest) = read(length[0] as usize).and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            return;
        };
        cache.clock += 1;
        let used = cache.clock;
        cache.entries.insert(
            (stamp[0], stamp[1], algorithm[0]),
            Entry {
                stamp,
                recorded_ns,
                used,
                digest,
            },
        );
    }
}

/// The file contents: the header (magic and generation), then the most
/// recently used digest last, so a file read back in order ends with the
/// newest.
fn encode(cache: &Cache) -> Vec<u8> {
    let mut entries: Vec<(&(i128, i128, u8), &Entry)> = cache.entries.iter().collect();
    entries.sort_unstable_by_key(|(_, entry)| entry.used);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&cache.generation.to_be_bytes());
    out.reserve(entries.len() * 130);
    for ((_, _, algorithm), entry) in entries {
        if entry.digest.len() > 255 {
            continue;
        }
        out.push(*algorithm);
        for value in entry.stamp {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(&entry.recorded_ns.to_be_bytes());
        out.push(entry.digest.len() as u8);
        out.extend_from_slice(entry.digest.as_bytes());
    }
    out
}

#[cfg(test)]
#[path = "hash_cache_tests.rs"]
mod tests;
