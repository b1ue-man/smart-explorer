//! The digests inside the blocks of the cache file (`hash_cache_file.rs`
//! frames them): one record is the algorithm, the five stamp values and the
//! moment the digest was taken (16 bytes each, big endian), the digest's
//! length and its text. Only `std` is used here, because the standalone
//! agent compiles `hash_cache.rs` too.
use std::path::Path;

use super::file::{self, BLOCK_RECORDS};
use super::{Cache, Entry, Key};

/// How many times digests were encoded for the file (tests: a process that
/// gave up writing encodes nothing).
#[cfg(test)]
pub(super) static ENCODES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One decoded record: key, stamp, moment, digest.
type Record = (Key, [i128; 5], i128, String);

fn push_record(out: &mut Vec<u8>, algorithm: u8, entry: &Entry) -> bool {
    if entry.digest.len() > 255 {
        return false;
    }
    out.push(algorithm);
    for value in entry.stamp {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.extend_from_slice(&entry.recorded_ns.to_be_bytes());
    out.push(entry.digest.len() as u8);
    out.extend_from_slice(entry.digest.as_bytes());
    true
}

/// The records of `entries`, in the order given, framed into blocks of at
/// most `BLOCK_RECORDS`; with the number of blocks and of records.
fn blocks_of<'a>(entries: impl Iterator<Item = (&'a Key, &'a Entry)>) -> (Vec<u8>, usize, usize) {
    #[cfg(test)]
    ENCODES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut out = Vec::new();
    let mut payload = Vec::new();
    let (mut in_block, mut blocks, mut records) = (0usize, 0usize, 0usize);
    for ((_, _, algorithm), entry) in entries {
        if !push_record(&mut payload, *algorithm, entry) {
            continue;
        }
        in_block += 1;
        records += 1;
        if in_block == BLOCK_RECORDS {
            file::frame(&mut out, in_block, &payload);
            payload.clear();
            in_block = 0;
            blocks += 1;
        }
    }
    if in_block > 0 {
        file::frame(&mut out, in_block, &payload);
        blocks += 1;
    }
    (out, blocks, records)
}

/// The whole file: the header (magic and generation), then every digest, the
/// most recently used last, so a file read back in order ends with the newest.
pub(super) fn whole_file(cache: &Cache) -> Vec<u8> {
    let mut entries: Vec<(&Key, &Entry)> = cache.entries.iter().collect();
    entries.sort_unstable_by_key(|(_, entry)| entry.used);
    let (blocks, _, _) = blocks_of(entries.into_iter());
    let mut out = file::MAGIC.to_vec();
    out.extend_from_slice(&cache.generation.to_be_bytes());
    out.extend_from_slice(&blocks);
    out
}

/// The blocks for the digests of `keys` (those still held), oldest use first;
/// with the number of blocks and of records.
pub(super) fn appended(cache: &Cache, keys: &[Key]) -> (Vec<u8>, usize, usize) {
    let mut entries: Vec<(&Key, &Entry)> = keys
        .iter()
        .filter_map(|key| cache.entries.get_key_value(key))
        .collect();
    entries.sort_unstable_by_key(|(_, entry)| entry.used);
    blocks_of(entries.into_iter())
}

/// The bytes of one block's payload, read front to back.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.0.len() < count {
            return None;
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Some(head)
    }

    fn int(&mut self) -> Option<i128> {
        Some(i128::from_be_bytes(self.take(16)?.try_into().ok()?))
    }
}

/// The `records` records of a payload, or `None` when it is not exactly that.
fn parse(payload: &[u8], records: usize) -> Option<Vec<Record>> {
    let mut input = Cursor(payload);
    let mut parsed = Vec::with_capacity(records);
    for _ in 0..records {
        let algorithm = input.take(1)?[0];
        let mut stamp = [0i128; 5];
        for slot in &mut stamp {
            *slot = input.int()?;
        }
        let recorded_ns = input.int()?;
        let length = usize::from(input.take(1)?[0]);
        let digest = String::from_utf8(input.take(length)?.to_vec()).ok()?;
        parsed.push(((stamp[0], stamp[1], algorithm), stamp, recorded_ns, digest));
    }
    input.0.is_empty().then_some(parsed)
}

/// Reads the cache file `path` into `cache`: the generation it names and
/// every digest of its whole blocks, the last ones as the most recently
/// used; at most `MAX_RESIDENT` stay (`Cache::put`). A file that is not of
/// this format names no generation. A damaged block ends the reading and
/// owes the file a rewrite from memory.
pub(super) fn load(cache: &mut Cache, path: &Path) {
    cache.generation = file::NO_GENERATION;
    let read = file::read_blocks(path, |payload, records| {
        let Some(parsed) = parse(payload, records) else {
            return false;
        };
        for (key, stamp, recorded_ns, digest) in parsed {
            cache.put(key, (stamp, recorded_ns, digest), false);
        }
        true
    });
    if let Some(read) = read {
        cache.generation = read.generation;
        cache.rewrite = !read.intact;
    }
}
