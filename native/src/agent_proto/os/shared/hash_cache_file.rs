//! The cache file's generation, its framing and the lock of its writers. The
//! file starts with `MAGIC` and a 16-byte generation, then blocks of digests
//! (`hash_cache_records.rs` encodes them). A block is one step of the file:
//! a tag, the number of records, the payload length and an FNV-1a checksum of
//! the payload, then the payload. A reader takes a block only when it is
//! complete and its checksum fits, and stops at the first one that is not, so
//! a write that broke off in the middle (a crash, a full disk) leaves no
//! half state: what follows the last whole block does not count, and the next
//! writer cuts it off before it appends.
//!
//! `clear_file` gives the file a new generation and no blocks; a writer
//! changes the file only while the file still names the generation the writer
//! loaded (`replace_if_generation`, `append_if_generation`). So a process that
//! holds digests of an older generation never writes them back. Digests that
//! were added are appended as new blocks; the file is rewritten as a whole
//! (`replace_if_generation`, compaction) only when it holds more blocks or
//! records than `append_if_generation` was allowed. The lock makes check and
//! change one step for every process of the device: an exclusive `File::lock`
//! on `hash-cache-v1.lock` beside the file, released when the lock file is
//! dropped or the process ends. Only `std` is used here, because the
//! standalone agent compiles `hash_cache.rs` too.
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const FILE_NAME: &str = "hash-cache-v1.bin";
const LOCK_NAME: &str = "hash-cache-v1.lock";
pub(super) const MAGIC: &[u8] = b"SEHC\x03";
/// Bytes before the first record: the magic and the generation.
pub(super) const HEADER_LEN: usize = MAGIC.len() + 16;
/// The generation of a file that is absent or not of this format.
pub(super) const NO_GENERATION: u128 = 0;

/// First byte of every block.
const BLOCK_TAG: u8 = 0xB1;
/// Tag, record count (u32), payload length (u32), checksum (u64).
pub(super) const BLOCK_HEADER_LEN: usize = 17;
/// Records per block: a block is built and read in one piece, so this bounds
/// the memory a reader or a writer needs for one (about 1.5 MiB at most).
pub(super) const BLOCK_RECORDS: usize = 4096;
/// The longest record: algorithm, five stamp values, the moment, the digest
/// length and a digest of 255 bytes.
const RECORD_MAX: usize = 1 + 5 * 16 + 16 + 1 + 255;
const PAYLOAD_MAX: usize = BLOCK_RECORDS * RECORD_MAX;

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// The outcome of `replace_if_generation`.
pub(super) enum Replaced {
    Written,
    /// The file names another generation: it was cleared since the writer
    /// loaded it, so the writer's digests are forgotten.
    Stale,
}

/// The outcome of `append_if_generation`.
pub(super) enum Appended {
    Written,
    /// As `Replaced::Stale`.
    Stale,
    /// The file holds too many blocks or records for the new ones: it is
    /// to be rewritten from memory (`replace_if_generation`).
    Compact,
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
        // The lock file holds no data; an existing one is kept as it is.
        .truncate(false)
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

/// The checksum of a block payload: FNV-1a, 64 bit.
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Appends the block of `records` records in `payload` to `out`.
pub(super) fn frame(out: &mut Vec<u8>, records: usize, payload: &[u8]) {
    out.push(BLOCK_TAG);
    out.extend_from_slice(&(records as u32).to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&checksum(payload).to_be_bytes());
    out.extend_from_slice(payload);
}

/// Record count, payload length and checksum of a block header; `None` for
/// bytes that cannot start a block of this format.
fn parse_header(head: &[u8; BLOCK_HEADER_LEN]) -> Option<(usize, usize, u64)> {
    if head[0] != BLOCK_TAG {
        return None;
    }
    let records = u32::from_be_bytes(head[1..5].try_into().ok()?) as usize;
    let length = u32::from_be_bytes(head[5..9].try_into().ok()?) as usize;
    let sum = u64::from_be_bytes(head[9..17].try_into().ok()?);
    (records <= BLOCK_RECORDS && length <= PAYLOAD_MAX).then_some((records, length, sum))
}

/// What `read_blocks` found.
pub(super) struct BlocksRead {
    pub(super) generation: u128,
    /// False when a block was damaged (checksum or payload), not merely
    /// missing its end: the file is then rewritten by the next write.
    pub(super) intact: bool,
}

/// Reads the cache file `path` block by block and hands each whole block's
/// payload and record count to `each`, which answers whether the payload was
/// well formed. Stops at the first block that is incomplete (not damaged: a
/// writer may be adding it), damaged, or refused by `each`. `None` when there
/// is no file or it lacks the header of this format.
pub(super) fn read_blocks(
    path: &Path,
    mut each: impl FnMut(&[u8], usize) -> bool,
) -> Option<BlocksRead> {
    let mut input = BufReader::new(File::open(path).ok()?);
    let mut head = [0u8; HEADER_LEN];
    input.read_exact(&mut head).ok()?;
    let generation = generation_in(&head)?;
    let mut payload = Vec::new();
    loop {
        let mut block = [0u8; BLOCK_HEADER_LEN];
        if input.read_exact(&mut block).is_err() {
            return Some(BlocksRead {
                generation,
                intact: true,
            });
        }
        let damaged = Some(BlocksRead {
            generation,
            intact: false,
        });
        let Some((records, length, sum)) = parse_header(&block) else {
            return damaged;
        };
        payload.resize(length, 0);
        if input.read_exact(&mut payload).is_err() {
            return Some(BlocksRead {
                generation,
                intact: true,
            });
        }
        if checksum(&payload) != sum || !each(&payload, records) {
            return damaged;
        }
    }
}

/// How far the whole blocks of an open cache file reach.
struct Extent {
    /// The offset after the last whole block.
    end: u64,
    blocks: usize,
    records: usize,
}

/// Walks the block headers of `file` (`len` bytes) without reading payloads.
fn extent(file: &mut File, len: u64) -> io::Result<Extent> {
    let mut found = Extent {
        end: HEADER_LEN as u64,
        blocks: 0,
        records: 0,
    };
    while found.end + BLOCK_HEADER_LEN as u64 <= len {
        file.seek(SeekFrom::Start(found.end))?;
        let mut head = [0u8; BLOCK_HEADER_LEN];
        file.read_exact(&mut head)?;
        let Some((records, length, _)) = parse_header(&head) else {
            break;
        };
        let next = found.end + (BLOCK_HEADER_LEN + length) as u64;
        if next > len {
            break;
        }
        found.end = next;
        found.blocks += 1;
        found.records += records;
    }
    Ok(found)
}

/// Appends `blocks` (`added_blocks` blocks of `added_records` records,
/// framed by `frame`) to the cache file of `folder`, unless the file names a
/// generation other than `expected` (`Stale`) or would then hold more than
/// `max_blocks` blocks or `max_records` records (`Compact`). Whatever follows
/// the last whole block (the remains of a write that broke off) is cut off
/// first. A write that fails cuts its own start off again.
pub(super) fn append_if_generation(
    folder: &Path,
    expected: u128,
    (blocks, added_blocks, added_records): (&[u8], usize, usize),
    (max_blocks, max_records): (usize, usize),
) -> io::Result<Appended> {
    let _lock = lock(folder)?;
    if generation_on_disk(folder) != expected {
        return Ok(Appended::Stale);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(folder.join(FILE_NAME))?;
    let len = file.metadata()?.len();
    let found = extent(&mut file, len)?;
    if found.blocks + added_blocks > max_blocks || found.records + added_records > max_records {
        return Ok(Appended::Compact);
    }
    if len > found.end {
        file.set_len(found.end)?;
    }
    file.seek(SeekFrom::Start(found.end))?;
    if let Err(error) = file.write_all(blocks).and_then(|()| file.flush()) {
        let _ = file.set_len(found.end);
        return Err(error);
    }
    Ok(Appended::Written)
}
