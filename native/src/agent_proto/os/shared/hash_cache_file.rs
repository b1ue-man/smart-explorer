//! The cache file's generation and the lock of its writers. The file starts
//! with `MAGIC` and a 16-byte generation, then the digests (`hash_cache.rs`).
//! `clear_file` gives the file a new generation and no digests; a writer
//! replaces the file only while the file still names the generation the
//! writer loaded (`replace_if_generation`). So a process that holds digests of
//! an older generation never writes them back. The lock makes that check and
//! the replacement one step for every process of the device: an exclusive
//! `File::lock` on `hash-cache-v1.lock` beside the file, released when the
//! lock file is dropped or the process ends. Only `std` is used here, because
//! the standalone agent compiles `hash_cache.rs` too.
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const FILE_NAME: &str = "hash-cache-v1.bin";
const LOCK_NAME: &str = "hash-cache-v1.lock";
pub(super) const MAGIC: &[u8] = b"SEHC\x02";
/// Bytes before the first record: the magic and the generation.
pub(super) const HEADER_LEN: usize = MAGIC.len() + 16;
/// The generation of a file that is absent or not of this format.
pub(super) const NO_GENERATION: u128 = 0;

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// The outcome of `replace_if_generation`.
pub(super) enum Replaced {
    Written,
    /// The file names another generation: it was cleared since the writer
    /// loaded it, so the writer's digests are forgotten.
    Stale,
}

/// A generation that differs from every earlier one of this device in
/// practice (clock, process and a counter); never `NO_GENERATION`.
pub(super) fn new_generation() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let serial = u128::from(SERIAL.fetch_add(1, Ordering::Relaxed));
    (nanos ^ (u128::from(std::process::id()) << 64) ^ (serial << 96)) | 1
}

/// The generation named by the bytes of a cache file, `None` when they lack
/// the header of this format or name `NO_GENERATION`.
pub(super) fn generation_in(bytes: &[u8]) -> Option<u128> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let generation = u128::from_be_bytes(rest.get(..16)?.try_into().ok()?);
    (generation != NO_GENERATION).then_some(generation)
}

/// The generation the cache file of `folder` names; `NO_GENERATION` when
/// there is no file or it lacks the header of this format.
pub(super) fn generation_on_disk(folder: &Path) -> u128 {
    let mut head = [0u8; HEADER_LEN];
    let read = File::open(folder.join(FILE_NAME)).and_then(|mut file| file.read_exact(&mut head));
    match read {
        Ok(()) => generation_in(&head).unwrap_or(NO_GENERATION),
        Err(_) => NO_GENERATION,
    }
}

/// The exclusive lock of the cache of `folder`, held until the returned file
/// is dropped.
pub(super) fn lock(folder: &Path) -> io::Result<File> {
    std::fs::create_dir_all(folder)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(folder.join(LOCK_NAME))?;
    file.lock()?;
    Ok(file)
}

/// Gives the cache file of `folder` a new generation and no digests, and
/// returns the generation.
pub(super) fn clear_file(folder: &Path) -> io::Result<u128> {
    let _lock = lock(folder)?;
    let generation = new_generation();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&generation.to_be_bytes());
    replace(folder, &bytes)?;
    Ok(generation)
}

/// Replaces the cache file of `folder` with `bytes`, unless the file names a
/// generation other than `expected`.
pub(super) fn replace_if_generation(
    folder: &Path,
    expected: u128,
    bytes: &[u8],
) -> io::Result<Replaced> {
    let _lock = lock(folder)?;
    if generation_on_disk(folder) != expected {
        return Ok(Replaced::Stale);
    }
    replace(folder, bytes)?;
    Ok(Replaced::Written)
}

/// Writes `bytes` to a staged file and renames it over the cache file.
fn replace(folder: &Path, bytes: &[u8]) -> io::Result<()> {
    let path = folder.join(FILE_NAME);
    let staged = folder.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    if let Err(error) =
        std::fs::write(&staged, bytes).and_then(|()| std::fs::rename(&staged, &path))
    {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(())
}
