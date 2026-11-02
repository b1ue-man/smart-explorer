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
//! FAT, exFAT, FUSE, SMB/CIFS, 9p or optical media; on Windows only local
//! NTFS and ReFS, no network volume): a file without a stamp is never
//! cached. A run that asks for a fresh read (`digest_fresh`, the checksum
//! compare mode of a local sync side) never takes a stored digest, and
//! stores the one it read.
//!
//! The process holds at most `MAX_RESIDENT` digests in memory at any moment,
//! also during a long walk that writes only at its end: a digest added above
//! that bound makes the least recently used ones leave (`Cache::trim`, down
//! to `MAX_ENTRIES`; `used` is moved by every hit and every store). The span
//! `TRIM_SLACK` above `MAX_ENTRIES` keeps the cost of a trim, one pass over
//! the entries, to once per `MAX_ENTRIES / 16` additions, so at most 16 steps
//! per added digest. The file holds at most the same number of records (plus
//! the ones a rewrite has not yet dropped) and the generation of the file
//! (`hash_cache_file.rs`). A cache that cannot be read or written costs only
//! the reuse, never a result.
//!
//! New digests are collected in memory and written together (`flush`: at
//! most every `FLUSH_INTERVAL`; `flush_now`: at the end of a walk or a
//! connection), under the file's lock and outside the cache's, so no thread
//! waits for a write. A write appends the new digests as blocks
//! (`hash_cache_records.rs`), so its cost follows what is new, not the size of
//! the file; the whole file is rewritten from memory, as one atomic
//! replacement, only for the first write, after a damaged block, and when
//! the file would otherwise hold more than `MAX_BLOCKS` blocks or
//! `MAX_RESIDENT` records. A write that breaks off leaves nothing that
//! counts: a reader takes whole blocks with a fitting checksum only. A hit
//! moves `used` in memory only; the file keeps the order of its blocks, which
//! a rewrite replaces by the order of use.
//!
//! A write that fails because the process may not write the folder (twice in
//! a row: `CONFIRMED_DENIALS`; a confined agent whose folder lies outside its
//! root, `--serve-root`) ends the writing for this process: digests stay in
//! memory, nothing is collected or encoded for the file any more. Any other
//! failure leaves the digests due for the next write. The folder that holds
//! the file is a protected omission of the agent's walks
//! (`is_cache_folder`, `walk_kept_out.rs`): the file changes at the end of
//! every walk, so syncing it would transfer it each time, and a missing
//! counterpart is never read as a deletion.
//!
//! The cache lives in the app's data folder (`support_dirs::app_data_dir`, on
//! Android the host's) or, in the standalone agent, in its cache folder
//! (`$XDG_CACHE_HOME/se-agent` or `~/.cache/se-agent`,
//! `se-agent/src/support_dirs.rs`).
//!
//! `clear` forgets every digest of this process and gives the file a new
//! generation, so a writer that still holds an older generation drops its
//! digests instead of writing them back (`write_pending`). A process the app
//! cannot reach (another command, a second instance) sees the new generation
//! within `GENERATION_CHECK` and forgets its digests then
//! (`forget_if_cleared`); it never writes them back. The app clears a running
//! background worker of the same account through its IPC at once
//! (`daemon::clear_hash_cache`, `se doctor --clear-hash-cache`).
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "hash_cache_file.rs"]
mod file;
use file::{FILE_NAME, NO_GENERATION};
#[path = "hash_cache_records.rs"]
mod records;
#[path = "hash_cache_write.rs"]
mod write;
#[cfg(any(test, feature = "test-support"))]
use write::take_pending;
use write::write_pending;

/// Digests kept after a trim (a record is 130 bytes in the file for an MD5
/// digest, 162 for a SHA-256 one).
const MAX_ENTRIES: usize = 500_000;
/// How far above `MAX_ENTRIES` the entries in memory may reach before the
/// least recently used ones leave: a trim passes over all entries, and this
/// span spreads that pass over `MAX_ENTRIES / 16` additions.
const TRIM_SLACK: usize = MAX_ENTRIES / 16;
/// The most digests in memory, and the most records in the file, at any time.
const MAX_RESIDENT: usize = MAX_ENTRIES + TRIM_SLACK;
/// The most blocks the file holds before it is rewritten: what a rewrite
/// writes, plus 256 appended blocks (one per walk or connection end).
const MAX_BLOCKS: usize = MAX_RESIDENT / file::BLOCK_RECORDS + 1 + 256;
/// Failures to write for lack of permission, in a row, after which the
/// process stops writing: one refusal can be a file another program holds.
const CONFIRMED_DENIALS: u8 = 2;
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

type Key = (i128, i128, u8);

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
    /// The digests added since they were last handed to a write.
    unwritten: HashSet<Key>,
    /// The whole file is owed from memory (a damaged block, a failed rewrite).
    rewrite: bool,
    /// The generation of the file the digests belong to (`NO_GENERATION`: none).
    generation: u128,
    /// A thread is writing the file (outside the lock).
    writing: bool,
    /// Writes refused for lack of permission, in a row.
    denied: u8,
    /// Writing was given up for this process (`CONFIRMED_DENIALS`).
    write_off: bool,
    /// When the file's generation was last compared with `generation`.
    generation_checked: Option<Instant>,
    last_write: Option<Instant>,
    clock: u64,
    entries: HashMap<Key, Entry>,
}

impl Cache {
    /// A write is owed: digests were added, or the file is to be rewritten.
    fn owes_write(&self) -> bool {
        !self.write_off && (self.rewrite || !self.unwritten.is_empty())
    }

    /// Stores a digest as the most recently used one (`track`: it is owed to
    /// the file), and lets the least recently used leave above `MAX_RESIDENT`.
    fn put(
        &mut self,
        key: Key,
        (stamp, recorded_ns, digest): ([i128; 5], i128, String),
        track: bool,
    ) {
        self.clock += 1;
        let used = self.clock;
        self.entries.insert(
            key,
            Entry {
                stamp,
                recorded_ns,
                used,
                digest,
            },
        );
        if track && self.folder.is_some() && !self.write_off {
            self.unwritten.insert(key);
        }
        if self.entries.len() > MAX_RESIDENT {
            self.trim();
        }
    }

    /// Keeps the `MAX_ENTRIES` most recently used digests.
    fn trim(&mut self) {
        let mut used: Vec<u64> = self.entries.values().map(|entry| entry.used).collect();
        let (_, oldest_kept, _) =
            used.select_nth_unstable_by(MAX_ENTRIES - 1, |left, right| right.cmp(left));
        let oldest_kept = *oldest_kept;
        self.entries.retain(|_, entry| entry.used >= oldest_kept);
        let entries = &self.entries;
        self.unwritten.retain(|key| entries.contains_key(key));
    }

    /// Forgets every digest and what was owed for them.
    fn forget(&mut self) {
        self.entries.clear();
        self.unwritten.clear();
        self.rewrite = false;
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The cache is one per process: every test that configures, clears or
/// relies on its contents takes this turn first.
#[cfg(any(test, feature = "test-support"))]
pub fn test_turn() -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
pub fn configure(folder: Option<PathBuf>) {
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
pub fn configure_once(folder: impl FnOnce() -> Option<PathBuf>) {
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
    if cfg!(any(test, feature = "test-support")) {
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
            records::load(cache, &folder.join(FILE_NAME));
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
    cache.forget();
    records::load(cache, &folder.join(FILE_NAME));
}

/// The digest (`algorithm`: `ops_types::digest` code) of the regular file
/// whose current status is `stamp` (`local_platform::file_stamp` or
/// `path_stamp`): the stored one while the status is unchanged and not racy,
/// else `compute()`, stored when the file kept its status (`after` is read
/// once the digest was computed).
pub fn digest(
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
pub fn digest_fresh(
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
            cache.put(key, (stamp, recorded_ns, digest.clone()), true);
        });
    }
    Ok(digest)
}

/// Writes the digests added since the last write, unless the last write was
/// less than `FLUSH_INTERVAL` ago (a request that may repeat per file).
pub fn flush() {
    write_pending(false);
}

/// Writes the digests added since the last write now (the end of a walk, of
/// a connection or of a run).
pub fn flush_now() {
    write_pending(true);
}

/// Forgets every digest of this process and gives the cache file of `folder`
/// a new generation without digests (`file::clear_file`). A background worker
/// keeps its own memory: it is cleared through its IPC.
pub fn clear(folder: &Path) -> io::Result<()> {
    let generation = file::clear_file(folder)?;
    if let Some(cache) = lock().as_mut() {
        cache.forget();
        if cache.folder.as_deref() == Some(folder) {
            cache.generation = generation;
            cache.loaded = true;
            // The folder can be written: a refusal of earlier is over.
            cache.denied = 0;
            cache.write_off = false;
        } else {
            // Another folder's cache: its own file is read again when used.
            cache.generation = NO_GENERATION;
            cache.loaded = false;
        }
    }
    Ok(())
}

/// Whether `path` is the folder that holds this process's cache file. Walks
/// of the agent leave it out as a protected omission (`walk_kept_out.rs`):
/// the file is replaced or extended at the end of every walk, and the walk
/// must neither transfer it nor read its absence on the other side as a
/// deletion. `path` is the folder as the walk reached it: it is the folder
/// itself, or has its name and resolves to it (a link on the way, another
/// spelling).
pub fn is_cache_folder(path: &Path) -> bool {
    let folder = {
        let mut guard = lock();
        let cache = guard.get_or_insert_with(|| Cache {
            folder: default_folder(),
            configured: true,
            ..Cache::default()
        });
        let Some(folder) = cache.folder.as_deref() else {
            return false;
        };
        if path == folder {
            return true;
        }
        if folder
            .file_name()
            .zip(path.file_name())
            .is_none_or(|(held, walked)| !held.eq_ignore_ascii_case(walked))
        {
            return false;
        }
        folder.to_path_buf()
    };
    match (std::fs::canonicalize(path), std::fs::canonicalize(&folder)) {
        (Ok(walked), Ok(held)) => walked == held,
        _ => false,
    }
}

/// The digests in memory (tests).
#[cfg(any(test, feature = "test-support"))]
pub fn resident_entries() -> usize {
    lock().as_ref().map_or(0, |cache| cache.entries.len())
}

/// The error every write fails with (tests): a refused permission cannot be
/// made on demand on a runner that is root.
#[cfg(any(test, feature = "test-support"))]
static FORCED_WRITE_ERROR: Mutex<Option<io::ErrorKind>> = Mutex::new(None);

#[cfg(any(test, feature = "test-support"))]
fn forced_write_error() -> Option<io::ErrorKind> {
    *FORCED_WRITE_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "hash_cache_append_tests.rs"]
mod append_tests;
#[cfg(test)]
#[path = "hash_cache_tests.rs"]
mod tests;
