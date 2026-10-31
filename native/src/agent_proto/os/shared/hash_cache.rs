//! Persistent digest cache on the side that stores the files (E9): the SSH
//! agent, a Share host and a local sync side keep the digests they computed,
//! keyed by the file's status (`local_platform::change_stamp`: device,
//! inode, size, modification and status-change time in nanoseconds) and the
//! algorithm. A digest is used again only while that status is unchanged.
//!
//! Racy timestamps (Git, `Documentation/technical/racy-git.txt`): a file
//! changed within the time granularity of the moment its digest was taken
//! can change again with the same times. So a digest counts only when the
//! file's modification and change times lie more than `RACY_SLACK` before
//! the moment the digest was taken (`recorded_ns`); otherwise it is computed
//! again each time until the file is old enough.
//!
//! The file holds at most `MAX_ENTRIES` digests, the most recently used
//! first; a side without a change time (Windows: `std::fs::Metadata` has
//! none) caches nothing. A cache that cannot be read or written costs only
//! the reuse, never a result. The agent keeps its file in its cache folder
//! (`$XDG_CACHE_HOME/se-agent` or `~/.cache/se-agent`), the app in its data
//! folder (`configure`).
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8] = b"SEHC\x01";
/// Digests kept (about 100 bytes each in the file).
const MAX_ENTRIES: usize = 500_000;
/// Times closer than this to the moment of hashing are "racy" (two seconds
/// cover the coarsest common time granularity, FAT).
const RACY_SLACK_NS: i128 = 2_000_000_000;
const FILE_NAME: &str = "hash-cache-v1.bin";

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
    dirty: bool,
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

/// Keeps the cache file in `folder` (the app: its data folder); `None`
/// keeps digests for this process only.
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

/// `configure`, unless a folder was configured already (the app sets its
/// data folder before every walk that hashes).
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

/// The agent's cache folder, from its environment (none in tests).
fn agent_folder() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?;
    Some(base.join("se-agent"))
}

fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut guard = lock();
    let cache = guard.get_or_insert_with(|| Cache {
        folder: agent_folder(),
        configured: true,
        ..Cache::default()
    });
    if !cache.loaded {
        cache.loaded = true;
        if let Some(folder) = cache.folder.clone() {
            load(cache, &folder.join(FILE_NAME));
        }
    }
    f(cache)
}

/// The digest (`algorithm`: `ops_types::digest` code) of the regular file
/// whose current status is `metadata`: the stored one while the status is
/// unchanged and not racy, else `compute()`, stored when the file kept its
/// status (`after` is read once the digest was computed).
pub(crate) fn digest(
    metadata: &std::fs::Metadata,
    algorithm: u8,
    compute: impl FnOnce() -> io::Result<String>,
    after: impl FnOnce() -> io::Result<std::fs::Metadata>,
) -> io::Result<String> {
    let Some(stamp) = super::local_platform::change_stamp(metadata) else {
        return compute();
    };
    let key = (stamp[0], stamp[1], algorithm);
    let hit = with_cache(|cache| {
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
    });
    if let Some(digest) = hit {
        return Ok(digest);
    }
    let recorded_ns = now_ns();
    let digest = compute()?;
    let unchanged = after()
        .ok()
        .and_then(|after| super::local_platform::change_stamp(&after))
        .is_some_and(|after| after == stamp);
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
        });
    }
    Ok(digest)
}

/// Writes the cache file when digests were added (after a walk).
pub(crate) fn flush() {
    let mut guard = lock();
    let Some(cache) = guard.as_mut() else {
        return;
    };
    let Some(folder) = cache.folder.clone().filter(|_| cache.dirty) else {
        return;
    };
    if cache.entries.len() > MAX_ENTRIES {
        let mut used: Vec<u64> = cache.entries.values().map(|entry| entry.used).collect();
        used.sort_unstable_by(|left, right| right.cmp(left));
        let oldest_kept = used[MAX_ENTRIES - 1];
        cache.entries.retain(|_, entry| entry.used >= oldest_kept);
    }
    if save(cache, &folder).is_ok() {
        cache.dirty = false;
    }
}

fn load(cache: &mut Cache, path: &Path) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Some(mut input) = bytes.strip_prefix(MAGIC) else {
        return;
    };
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

fn save(cache: &Cache, folder: &Path) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let mut entries: Vec<(&(i128, i128, u8), &Entry)> = cache.entries.iter().collect();
    // Oldest first, so a file read back in order ends with the newest.
    entries.sort_unstable_by_key(|(_, entry)| entry.used);
    let mut out = MAGIC.to_vec();
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
    let path = folder.join(FILE_NAME);
    let staged = folder.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    if let Err(error) = std::fs::write(&staged, &out).and_then(|()| std::fs::rename(&staged, &path))
    {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
#[path = "hash_cache_tests.rs"]
mod tests;
