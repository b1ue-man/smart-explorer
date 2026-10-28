//! Batched chunk crypto (G4 MT1/MT2): the byte format stays the per-chunk
//! one, every chunk is authenticated before a batch is released, and the
//! `speed_task_vault_*` benchmarks measure the stream throughput.
use std::io::{ErrorKind, Read, Write};

use super::super::cipher_combo::{file_size, CipherCombo, CHUNK_PAYLOAD};
use super::super::content_stream::{ContentCryptor, DecryptingReader, EncryptingWriter};
use super::super::keys::{Masterkey, MasterkeyCell};
use super::BATCH_CHUNKS;

const COMBOS: [CipherCombo; 2] = [CipherCombo::SivGcm, CipherCombo::SivCtrMac];

fn cryptor(combo: CipherCombo) -> ContentCryptor {
    ContentCryptor::new(MasterkeyCell::new(Masterkey::generate().unwrap()), combo)
}

fn encrypt(data: &[u8], cryptor: &ContentCryptor, piece: usize) -> Vec<u8> {
    let mut writer = EncryptingWriter::new(Vec::new(), cryptor.clone()).unwrap();
    for part in data.chunks(piece.max(1)) {
        writer.write_all(part).unwrap();
    }
    writer.finish().unwrap()
}

fn decrypt(file: &[u8], cryptor: &ContentCryptor) -> std::io::Result<Vec<u8>> {
    let mut reader = DecryptingReader::new(file, cryptor.clone())?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

#[test]
fn speed_task_vault_batched_stream_keeps_the_per_chunk_format() {
    for combo in COMBOS {
        let cryptor = cryptor(combo);
        // Several batches plus a partial chunk, written in odd pieces.
        let clear = BATCH_CHUNKS * CHUNK_PAYLOAD * 2 + 3 * CHUNK_PAYLOAD + 123;
        let data: Vec<u8> = (0..clear).map(|index| (index % 251) as u8).collect();
        let file = encrypt(&data, &cryptor, 70_001);
        assert_eq!(
            file.len() as u64,
            file_size(clear as u64, combo),
            "{combo:?}"
        );
        assert_eq!(decrypt(&file, &cryptor).unwrap(), data, "{combo:?}");
        // Every chunk on its own opens with the single-chunk primitive: the
        // batch changed how chunks are produced, not what they are.
        let header = cryptor
            .decrypt_header(&file[..combo.header_size()])
            .unwrap();
        for (index, chunk) in file[combo.header_size()..]
            .chunks(combo.chunk_size())
            .enumerate()
        {
            let plain = cryptor.decrypt_chunk(&header, index as u64, chunk).unwrap();
            let start = index * CHUNK_PAYLOAD;
            assert_eq!(&plain[..], &data[start..start + plain.len()]);
        }
        // Each chunk has its own random nonce.
        let size = combo.chunk_size();
        let body = &file[combo.header_size()..];
        let nonce = combo.nonce_len();
        assert_ne!(&body[..nonce], &body[size..size + nonce]);
    }
}

#[test]
fn speed_task_vault_flush_writes_every_full_chunk_and_keeps_a_partial_one() {
    let cryptor = cryptor(CipherCombo::SivGcm);
    let mut writer = EncryptingWriter::new(Vec::new(), cryptor.clone()).unwrap();
    writer.write_all(&vec![1u8; 2 * CHUNK_PAYLOAD + 5]).unwrap();
    writer.flush().unwrap();
    let file = writer.finish().unwrap();
    let combo = CipherCombo::SivGcm;
    assert_eq!(
        file.len() as u64,
        file_size((2 * CHUNK_PAYLOAD + 5) as u64, combo)
    );
    assert_eq!(
        decrypt(&file, &cryptor).unwrap(),
        vec![1u8; 2 * CHUNK_PAYLOAD + 5]
    );
}

#[test]
fn speed_task_vault_damaged_chunk_inside_a_batch_releases_nothing_of_it() {
    for combo in COMBOS {
        let cryptor = cryptor(combo);
        let data = vec![9u8; 8 * CHUNK_PAYLOAD];
        let file = encrypt(&data, &cryptor, 1 << 20);
        // Chunk 0 is the first (single-chunk) batch, chunks 1-2 the second;
        // damage chunk 2: nothing of chunk 1 may come out.
        let mut damaged = file.clone();
        damaged[combo.header_size() + 2 * combo.chunk_size() + 30] ^= 0x01;
        let mut reader = DecryptingReader::new(&damaged[..], cryptor.clone()).unwrap();
        let mut first = vec![0u8; CHUNK_PAYLOAD];
        reader.read_exact(&mut first).unwrap();
        assert_eq!(first, data[..CHUNK_PAYLOAD]);
        let mut buffer = [0u8; 64];
        assert_eq!(
            reader.read(&mut buffer).unwrap_err().kind(),
            ErrorKind::InvalidData,
            "{combo:?}"
        );
        assert!(reader.read(&mut buffer).is_err());
    }
}

/// `BENCH vault_<combo>_<case> files_per_s=<n> mib_per_s=<n>` for the G5
/// cases: 5000 × 4 KiB and 1000 × 1 MiB encrypted through `EncryptingWriter`
/// and decrypted through `DecryptingReader` in memory; 1 × 2 GiB encrypted
/// into a sink (its ciphertext is not kept).
#[test]
#[ignore = "benchmark: cargo test speed_task_vault -- --ignored --nocapture"]
fn speed_task_vault_stream_throughput() {
    for combo in COMBOS {
        let cryptor = cryptor(combo);
        for (case, files, size) in [
            ("5000x4KiB", 5000usize, 4096usize),
            ("1000x1MiB", 1000, 1 << 20),
        ] {
            let data = vec![0x5au8; size];
            let started = std::time::Instant::now();
            for _ in 0..files {
                let file = encrypt(&data, &cryptor, 256 * 1024);
                let mut reader = DecryptingReader::new(&file[..], cryptor.clone()).unwrap();
                std::io::copy(&mut reader, &mut std::io::sink()).unwrap();
            }
            report(
                &format!("vault_{combo:?}_{case}"),
                files,
                files * size,
                started,
            );
        }
        let block = vec![0x5au8; 1 << 20];
        let started = std::time::Instant::now();
        let mut writer = EncryptingWriter::new(std::io::sink(), cryptor.clone()).unwrap();
        for _ in 0..2048 {
            writer.write_all(&block).unwrap();
        }
        writer.finish().unwrap();
        report(&format!("vault_{combo:?}_1x2GiB"), 1, 2048 << 20, started);
    }
}

fn report(case: &str, files: usize, bytes: usize, started: std::time::Instant) {
    let seconds = started.elapsed().as_secs_f64().max(1e-9);
    println!(
        "BENCH {case} files_per_s={:.1} mib_per_s={:.1}",
        files as f64 / seconds,
        bytes as f64 / (1024.0 * 1024.0) / seconds
    );
}
