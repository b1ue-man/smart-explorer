//! Signals of the sealed spool of vault drives (VLT-M12, stage `vault_unit`,
//! job linux-core: unit): random access across chunk boundaries against a
//! plain model (write, overwrite, gap, truncate, extend), a fresh nonce for
//! every rewritten chunk, no cleartext on disk, and chunks that moved fail.
use std::fs;
use std::io;
use std::path::Path;

use crate::cryptomator::{ciphertext_size, CipherCombo, CHUNK_PAYLOAD};
use crate::mount::{SpoolFile, SpoolStorage};

use super::sealed_spool::SealedSpoolStorage;
use super::spool_key::SpoolKey;

const MARKER: &[u8] = b"KLARTEXT-MARKER-VLT-M12";
const HEADER: usize = 68;
const CHUNK: usize = CHUNK_PAYLOAD + 28;
const P: u64 = CHUNK_PAYLOAD as u64;

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// The plain model of a write: zero bytes fill a gap.
fn apply(model: &mut Vec<u8>, offset: u64, bytes: &[u8]) {
    let offset = offset as usize;
    if model.len() < offset + bytes.len() {
        model.resize(offset + bytes.len(), 0);
    }
    model[offset..offset + bytes.len()].copy_from_slice(bytes);
}

fn read_all(file: &mut dyn SpoolFile) -> Vec<u8> {
    let len = file.len().unwrap() as usize;
    let mut out = vec![0u8; len];
    assert_eq!(file.read_at(0, &mut out).unwrap(), len);
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn disk_len(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

#[test]
fn vault_task_sealed_spool_random_access_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("0123456789abcdef0123456789abcdef.spool");
    let key = SpoolKey::generate().unwrap();
    let storage = SealedSpoolStorage::new(&key).unwrap();
    let mut file = storage.create_new(&path).unwrap();
    let mut model: Vec<u8> = Vec::new();
    // A new spool is its header only (CryptoFS form of an empty file).
    assert_eq!(file.len().unwrap(), 0);
    assert_eq!(disk_len(&path), HEADER as u64);

    let steps: Vec<(u64, Vec<u8>)> = vec![
        (0, MARKER.repeat(3)),
        // Across the first chunk boundary.
        (P - 5, pattern(10, 1)),
        // Behind the end: the gap becomes zero bytes, the old last chunk
        // becomes full.
        (2 * P, pattern(P as usize + 7, 2)),
        // Overwrite across two boundaries.
        (3, pattern(2 * P as usize, 3)),
        // Far behind the end again.
        (5 * P + 11, MARKER.to_vec()),
    ];
    for (offset, bytes) in &steps {
        file.write_at(*offset, bytes).unwrap();
        apply(&mut model, *offset, bytes);
        assert_eq!(file.len().unwrap(), model.len() as u64);
        assert_eq!(read_all(&mut *file), model);
        let expected = HEADER as u64 + ciphertext_size(model.len() as u64, CipherCombo::SivGcm);
        assert_eq!(disk_len(&path), expected);
    }
    // Reads across a boundary, short at the end, nothing after it.
    let mut window = vec![0u8; 9];
    assert_eq!(file.read_at(P - 4, &mut window).unwrap(), 9);
    assert_eq!(window, model[(P - 4) as usize..(P + 5) as usize]);
    let end = model.len() as u64;
    assert_eq!(file.read_at(end - 3, &mut window).unwrap(), 3);
    assert_eq!(&window[..3], &model[model.len() - 3..]);
    assert_eq!(file.read_at(end, &mut window).unwrap(), 0);

    // Truncate inside a chunk and at a boundary, extend with zero bytes.
    for length in [3 * P + 100, 2 * P, 2 * P + 1, 4 * P + 3, 0, 17] {
        file.set_len(length).unwrap();
        model.resize(length as usize, 0);
        assert_eq!(file.len().unwrap(), length);
        assert_eq!(read_all(&mut *file), model);
        let expected = HEADER as u64 + ciphertext_size(length, CipherCombo::SivGcm);
        assert_eq!(disk_len(&path), expected);
    }
    file.sync_data().unwrap();
    assert!(!file.is_empty().unwrap());

    // Another handle reads what this one wrote.
    let mut reader = storage.open(&path, false).unwrap();
    assert_eq!(read_all(&mut *reader), model);
    file.write_at(17, b"tail").unwrap();
    apply(&mut model, 17, b"tail");
    assert_eq!(read_all(&mut *reader), model);

    // The same bytes written twice: the chunk is encrypted again with a
    // fresh nonce, the header stays.
    let same = pattern(17, 9);
    file.write_at(0, &same).unwrap();
    let first = fs::read(&path).unwrap();
    file.write_at(0, &same).unwrap();
    let second = fs::read(&path).unwrap();
    assert_eq!(first.len(), second.len());
    assert_eq!(&first[..HEADER], &second[..HEADER]);
    assert_ne!(
        &first[HEADER..HEADER + 12],
        &second[HEADER..HEADER + 12],
        "a rewritten chunk kept its nonce"
    );
    assert_ne!(&first[HEADER..], &second[HEADER..]);

    // No cleartext reaches the disk.
    file.write_at(0, &MARKER.repeat(3000)).unwrap();
    assert!(!contains(&fs::read(&path).unwrap(), MARKER));

    // Another spool key opens nothing.
    let other = SealedSpoolStorage::new(&SpoolKey::generate().unwrap()).unwrap();
    let refused = other.open(&path, false).err().unwrap();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);

    // A spool whose header never reached the disk has no content; the first
    // write gives it one.
    let late_path = dir.path().join("ffffffffffffffffffffffffffffffff.spool");
    fs::write(&late_path, b"").unwrap();
    let mut late = storage.open(&late_path, true).unwrap();
    assert_eq!(late.len().unwrap(), 0);
    let mut nothing = [0u8; 4];
    assert_eq!(late.read_at(0, &mut nothing).unwrap(), 0);
    late.write_at(0, b"x").unwrap();
    assert_eq!(read_all(&mut *late), b"x");

    // A name that exists is never created again.
    let taken = storage.create_new(&path).err().unwrap();
    assert_eq!(taken.kind(), io::ErrorKind::AlreadyExists);
}

#[test]
fn vault_task_sealed_spool_detects_block_swap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("00000000000000000000000000000001.spool");
    let storage = SealedSpoolStorage::new(&SpoolKey::generate().unwrap()).unwrap();
    let mut file = storage.create_new(&path).unwrap();
    let content: Vec<u8> = (0..4u8)
        .flat_map(|seed| pattern(CHUNK_PAYLOAD, seed))
        .chain(pattern(100, 7))
        .collect();
    file.write_at(0, &content).unwrap();
    file.sync_data().unwrap();
    drop(file);
    let original = fs::read(&path).unwrap();
    let at = |index: usize| HEADER + index * CHUNK;

    // Chunks 0 and 1 change places: both fail, chunk 2 still reads.
    let mut swapped = original.clone();
    swapped[at(0)..at(1)].copy_from_slice(&original[at(1)..at(2)]);
    swapped[at(1)..at(2)].copy_from_slice(&original[at(0)..at(1)]);
    fs::write(&path, &swapped).unwrap();
    let mut file = storage.open(&path, false).unwrap();
    let mut out = vec![0u8; 16];
    for offset in [0, P] {
        let error = file.read_at(offset, &mut out).unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidData,
            "chunk at {offset}"
        );
    }
    assert_eq!(file.read_at(2 * P, &mut out).unwrap(), 16);
    assert_eq!(
        &out[..],
        &content[2 * CHUNK_PAYLOAD..2 * CHUNK_PAYLOAD + 16]
    );

    // A chunk of another spool of the same drive (same content key, other
    // file key) fails at the same place.
    let other_path = dir.path().join("00000000000000000000000000000002.spool");
    let mut other = storage.create_new(&other_path).unwrap();
    other.write_at(0, &content).unwrap();
    drop(other);
    let foreign = fs::read(&other_path).unwrap();
    let mut mixed = original.clone();
    mixed[at(3)..at(4)].copy_from_slice(&foreign[at(3)..at(4)]);
    fs::write(&path, &mixed).unwrap();
    let mut file = storage.open(&path, false).unwrap();
    let error = file.read_at(3 * P, &mut out).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    // One flipped bit in a chunk fails; the bytes before it still read.
    let mut flipped = original.clone();
    flipped[at(1) + 40] ^= 0x01;
    fs::write(&path, &flipped).unwrap();
    let mut file = storage.open(&path, false).unwrap();
    assert_eq!(file.read_at(0, &mut out).unwrap(), 16);
    assert!(file.read_at(P + 3, &mut out).is_err());

    // A changed header opens nothing.
    let mut header = original;
    header[20] ^= 0x80;
    fs::write(&path, &header).unwrap();
    let refused = storage.open(&path, false).err().unwrap();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
}
