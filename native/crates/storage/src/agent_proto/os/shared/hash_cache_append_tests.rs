//! The digest cache's memory bound, its appending writes and its stop after
//! refused writes (`hash_cache.rs`, `hash_cache_file.rs`,
//! `hash_cache_write.rs`): digests are synthetic (no file is read), so the
//! tests run on every platform. The cache is one per process: each test takes
//! `test_turn`.
use super::*;
use crate::agent_proto::digest::MD5;
use std::io::Write;
use std::sync::atomic::Ordering;

/// The stamp of synthetic file `n`: old times, so the digest is not racy.
fn stamp(n: i128) -> [i128; 5] {
    [1, n, 10, 0, 0]
}

/// One MD5 record in the file: algorithm, five stamp values and the moment,
/// the length byte and 32 hex digits.
const RECORD: usize = 1 + 5 * 16 + 16 + 1 + 32;

/// Stores the digest of synthetic file `n` as a walk does after reading it.
fn store(n: i128) {
    let stamp = stamp(n);
    digest(Some(stamp), MD5, || Ok(format!("{n:032x}")), || Some(stamp)).unwrap();
}

/// Whether the digest of synthetic file `n` is held; never computes one.
fn held(n: i128) -> bool {
    let stamp = stamp(n);
    digest(
        Some(stamp),
        MD5,
        || Err(io::Error::other("not held")),
        || Some(stamp),
    )
    .is_ok()
}

fn cache_file(folder: &tempfile::TempDir) -> PathBuf {
    folder.path().join(FILE_NAME)
}

/// What the cache file holds: its whole blocks, their records, the length the
/// whole blocks add up to, and whether no block was damaged.
fn shape(folder: &tempfile::TempDir) -> (usize, usize, usize, bool) {
    let (mut blocks, mut records, mut length) = (0, 0, file::HEADER_LEN);
    let read = file::read_blocks(&cache_file(folder), |payload, count| {
        blocks += 1;
        records += count;
        length += file::BLOCK_HEADER_LEN + payload.len();
        true
    })
    .expect("a cache file of this format");
    (blocks, records, length, read.intact)
}

/// Makes every write of the cache fail with `kind` until dropped.
struct ForcedWriteError;

impl ForcedWriteError {
    fn new(kind: io::ErrorKind) -> Self {
        Self::set(Some(kind));
        Self
    }

    fn set(kind: Option<io::ErrorKind>) {
        *FORCED_WRITE_ERROR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = kind;
    }
}

impl Drop for ForcedWriteError {
    fn drop(&mut self) {
        Self::set(None);
    }
}

#[test]
fn abgleich_task_hash_cache_memory_stays_within_the_bound_during_a_long_walk() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    // A walk that hashes more files than the cache holds and writes only at
    // its end: one past the bound makes the least recently used leave.
    let total = MAX_RESIDENT as i128 + 10;
    let mut peak = 0;
    for n in 0..total {
        if n == MAX_RESIDENT as i128 - 1 {
            assert!(held(7), "key 7 is used again just before the trim");
        }
        store(n);
        peak = peak.max(resident_entries());
    }
    assert_eq!(peak, MAX_RESIDENT, "memory never went above the bound");
    assert_eq!(
        resident_entries(),
        MAX_ENTRIES + 9,
        "a trim keeps MAX_ENTRIES"
    );
    assert!(!held(0) && !held(8), "the least recently used left");
    assert!(
        held(7),
        "a digest used again stays; the order is use, not age"
    );
    assert!(held(total - 1), "the newest stays");
    {
        let guard = lock();
        let cache = guard.as_ref().unwrap();
        assert!(
            cache
                .unwritten
                .iter()
                .all(|key| cache.entries.contains_key(key)),
            "nothing is owed to the file for a digest that left"
        );
    }
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_file_holds_no_more_records_than_the_bound() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    for n in 0..MAX_RESIDENT as i128 + 10 {
        store(n);
    }
    flush_now();
    let (_, records, length, intact) = shape(&folder);
    assert!(intact);
    assert_eq!(
        records,
        resident_entries(),
        "the end of the walk wrote what is held"
    );
    assert_eq!(
        length as u64,
        std::fs::metadata(cache_file(&folder)).unwrap().len()
    );
    // More digests than the file has room for: appending would take it
    // above the bound, so it is rewritten from memory instead.
    for n in 1_000_000..1_000_000 + 40_000 {
        store(n);
    }
    flush_now();
    let (_, records, _, intact) = shape(&folder);
    assert!(intact && records <= MAX_RESIDENT, "{records} records");
    assert_eq!(records, resident_entries());
    // A new process holds no more than the bound either.
    configure(None);
    configure(Some(folder.path().to_path_buf()));
    assert!(held(1_000_000 + 39_999));
    assert!(resident_entries() <= MAX_RESIDENT);
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_a_write_appends_what_is_new_and_keeps_the_rest_of_the_file() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    for n in 0..1000 {
        store(n);
    }
    flush_now();
    let before = std::fs::read(cache_file(&folder)).unwrap();
    assert_eq!(
        before.len(),
        file::HEADER_LEN + file::BLOCK_HEADER_LEN + 1000 * RECORD
    );
    store(5_000);
    flush_now();
    let after = std::fs::read(cache_file(&folder)).unwrap();
    assert_eq!(
        after[..before.len()],
        before[..],
        "the old blocks were not touched"
    );
    assert_eq!(after.len(), before.len() + file::BLOCK_HEADER_LEN + RECORD);
    assert_eq!(shape(&folder), (2, 1001, after.len(), true));
    // A new process takes the blocks in order.
    configure(None);
    configure(Some(folder.path().to_path_buf()));
    assert!((0..1000).all(held) && held(5_000));
    // Hits move the order of use in memory only: nothing is written.
    flush_now();
    assert_eq!(std::fs::read(cache_file(&folder)).unwrap(), after);
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_rewrites_the_file_only_when_it_holds_too_many_blocks() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    store(0);
    flush_now();
    let generation = file::generation_on_disk(folder.path());
    assert_ne!(generation, file::NO_GENERATION);
    let (mut most, mut rewrites, mut last) = (0, 0, 0);
    for n in 1..MAX_BLOCKS as i128 + 5 {
        store(n);
        flush_now();
        let (blocks, records, _, intact) = shape(&folder);
        assert!(intact && blocks <= MAX_BLOCKS, "{blocks} blocks");
        assert_eq!(records as i128, n + 1, "no digest was lost");
        if blocks < last {
            rewrites += 1;
        }
        (last, most) = (blocks, most.max(blocks));
    }
    assert_eq!(most, MAX_BLOCKS);
    assert_eq!(
        rewrites, 1,
        "one rewrite when the block count reached its limit"
    );
    assert_eq!(
        file::generation_on_disk(folder.path()),
        generation,
        "a rewrite keeps the generation"
    );
    configure(None);
    configure(Some(folder.path().to_path_buf()));
    assert!((0..MAX_BLOCKS as i128 + 5).all(held));
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_a_writer_of_an_older_generation_never_appends() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    store(1);
    flush_now();
    assert!(std::fs::metadata(cache_file(&folder)).unwrap().len() > file::HEADER_LEN as u64);
    // Another process clears the file; this one still holds digest 1 and adds 2.
    let other = file::clear_file(folder.path()).unwrap();
    store(2);
    flush_now();
    assert_eq!(file::generation_on_disk(folder.path()), other);
    assert_eq!(
        std::fs::metadata(cache_file(&folder)).unwrap().len(),
        file::HEADER_LEN as u64,
        "neither the old nor the new digest was appended"
    );
    assert!(
        !held(1) && !held(2),
        "the digests of the older generation are forgotten"
    );
    configure(None);
}

/// The blocks of a record that a writer would append for synthetic file `n`.
fn block_of(n: i128) -> Vec<u8> {
    let mut other = Cache::default();
    let key = (1, n, MD5);
    other.put(key, (stamp(n), now_ns(), format!("{n:032x}")), false);
    records::appended(&other, &[key]).0
}

#[test]
fn abgleich_task_hash_cache_a_write_that_broke_off_leaves_no_half_state() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    for n in 0..10 {
        store(n);
    }
    flush_now();
    // A writer died after half of its block reached the file.
    let torn = block_of(99);
    let mut broken = std::fs::OpenOptions::new()
        .append(true)
        .open(cache_file(&folder))
        .unwrap();
    broken.write_all(&torn[..torn.len() / 2]).unwrap();
    drop(broken);
    // A new process takes the whole blocks and nothing of the half block.
    configure(None);
    configure(Some(folder.path().to_path_buf()));
    assert!((0..10).all(held));
    assert!(!held(99));
    // The next write cuts the remains off before it appends.
    store(10);
    flush_now();
    let (blocks, records, length, intact) = shape(&folder);
    assert_eq!((blocks, records, intact), (2, 11, true));
    assert_eq!(
        length as u64,
        std::fs::metadata(cache_file(&folder)).unwrap().len(),
        "no remains between or after the blocks"
    );
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_a_damaged_block_ends_the_reading_and_owes_a_rewrite() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    for n in 0..5 {
        store(n);
    }
    flush_now();
    store(5);
    flush_now();
    let mut bytes = std::fs::read(cache_file(&folder)).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(cache_file(&folder), bytes).unwrap();
    configure(None);
    configure(Some(folder.path().to_path_buf()));
    assert!((0..5).all(held), "the block before the damage counts");
    assert!(!held(5), "the damaged block does not");
    // The rewrite from memory repairs the file.
    store(6);
    flush_now();
    let (blocks, records, _, intact) = shape(&folder);
    assert_eq!(
        (blocks, records, intact),
        (1, 6, true),
        "rewritten as a whole"
    );
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_stops_encoding_after_confirmed_permission_denials() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let denied = ForcedWriteError::new(io::ErrorKind::PermissionDenied);
    let encoded = || records::ENCODES.load(Ordering::SeqCst);
    let start = encoded();
    store(1);
    flush_now();
    store(2);
    flush_now();
    assert_eq!(encoded() - start, 2, "both refusals were tried");
    for n in 3..60 {
        store(n);
        flush_now();
        flush();
    }
    assert_eq!(
        encoded() - start,
        2,
        "nothing is encoded after the second refusal"
    );
    assert!((1..60).all(held), "the digests stay in memory");
    // A clear that wrote the folder ends the refusal.
    drop(denied);
    clear(folder.path()).unwrap();
    store(100);
    flush_now();
    assert_eq!(encoded() - start, 3);
    assert_eq!(shape(&folder).1, 1);
    configure(None);
}

#[test]
fn abgleich_task_hash_cache_other_write_failures_keep_the_digests_owed() {
    let _turn = test_turn();
    let folder = tempfile::tempdir().unwrap();
    configure(Some(folder.path().to_path_buf()));
    let failing = ForcedWriteError::new(io::ErrorKind::Other);
    for n in 0..5 {
        store(n);
        flush_now();
    }
    assert!(!cache_file(&folder).exists());
    ForcedWriteError::set(None);
    flush_now();
    assert_eq!(shape(&folder).1, 5, "every digest reached the first file");
    // An append that fails owes its digests again.
    ForcedWriteError::set(Some(io::ErrorKind::Other));
    store(10);
    flush_now();
    assert_eq!(shape(&folder).1, 5);
    ForcedWriteError::set(None);
    flush_now();
    assert_eq!(shape(&folder).1, 6);
    drop(failing);
    configure(None);
}
