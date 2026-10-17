//! File contents: both empty-file forms (C1.7), the size formulas at the
//! chunk boundaries, streaming round trips, damaged chunks and locking.
use std::io::{ErrorKind, Read, Write};

use super::cipher_combo::{
    ciphertext_size, cleartext_file_size, cleartext_size, file_size, CipherCombo, CHUNK_PAYLOAD,
};
use super::content_stream::{
    decrypt_bytes, encrypt_bytes, ContentCryptor, DecryptingReader, EncryptingWriter,
};
use super::keys::{Masterkey, MasterkeyCell};
use super::FormatError;

const COMBOS: [CipherCombo; 2] = [CipherCombo::SivGcm, CipherCombo::SivCtrMac];

fn cryptor(combo: CipherCombo) -> ContentCryptor {
    ContentCryptor::new(MasterkeyCell::new(Masterkey::generate().unwrap()), combo)
}

fn read_all(bytes: &[u8], cryptor: &ContentCryptor) -> std::io::Result<Vec<u8>> {
    let mut reader = DecryptingReader::new(bytes, cryptor.clone())?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

fn write_all(data: &[u8], cryptor: &ContentCryptor, piece: usize) -> Vec<u8> {
    let mut writer = EncryptingWriter::new(Vec::new(), cryptor.clone()).unwrap();
    for part in data.chunks(piece.max(1)) {
        writer.write_all(part).unwrap();
    }
    writer.finish().unwrap()
}

#[test]
fn vault_task_empty_file_both_forms() {
    for combo in COMBOS {
        let cryptor = cryptor(combo);
        let header = combo.header_size();
        // The form se writes (CryptoFS): only the header.
        let empty = write_all(b"", &cryptor, 1);
        assert_eq!(empty.len(), header, "{combo:?}");
        assert_eq!(read_all(&empty, &cryptor).unwrap(), b"");
        assert_eq!(cleartext_file_size(empty.len() as u64, combo), Ok(0));
        assert_eq!(encrypt_bytes(&cryptor, b"").unwrap().len(), header);
        assert_eq!(&decrypt_bytes(&cryptor, &empty, 0).unwrap()[..], b"");

        // The form of CryptoLib's stream writer: header and one empty chunk
        // (68 + 28 or 88 + 48 bytes); read as empty, its tag still checked.
        let file_header = cryptor.new_header().unwrap();
        let mut lib_form = cryptor.encrypt_header(&file_header).unwrap();
        lib_form.extend(cryptor.encrypt_chunk(&file_header, 0, b"").unwrap());
        assert_eq!(lib_form.len(), header + combo.chunk_overhead());
        assert_eq!(read_all(&lib_form, &cryptor).unwrap(), b"");
        assert_eq!(&decrypt_bytes(&cryptor, &lib_form, 0).unwrap()[..], b"");
        assert_eq!(cleartext_file_size(lib_form.len() as u64, combo), Ok(0));
        let mut tampered = lib_form.clone();
        if let Some(last) = tampered.last_mut() {
            *last ^= 0x01;
        }
        assert_eq!(
            read_all(&tampered, &cryptor).unwrap_err().kind(),
            ErrorKind::InvalidData
        );

        // Shorter than a header: no encrypted file.
        assert!(read_all(&empty[..header - 1], &cryptor).is_err());
        assert_eq!(
            cleartext_file_size(header as u64 - 1, combo),
            Err(FormatError::InvalidSize)
        );
        // An unfinished writer that is dropped still leaves a valid empty
        // file (best effort, like `BufWriter`).
        let mut sink = Vec::new();
        drop(EncryptingWriter::new(&mut sink, cryptor.clone()).unwrap());
        assert_eq!(sink.len(), header);
        assert_eq!(read_all(&sink, &cryptor).unwrap(), b"");
    }
}

#[test]
fn vault_task_size_formulas_boundaries() {
    for combo in COMBOS {
        let payload = CHUNK_PAYLOAD as u64;
        let chunk = combo.chunk_size() as u64;
        let overhead = combo.chunk_overhead() as u64;
        let header = combo.header_size() as u64;
        assert_eq!(chunk, payload + overhead);
        assert_eq!(
            (header, overhead),
            match combo {
                CipherCombo::SivGcm => (68, 28),
                CipherCombo::SivCtrMac => (88, 48),
            }
        );
        for (clear, cipher) in [
            (0, 0),
            (1, 1 + overhead),
            (payload - 1, payload - 1 + overhead),
            (payload, chunk),
            (payload + 1, chunk + 1 + overhead),
            (2 * payload, 2 * chunk),
            (2 * payload + 5, 2 * chunk + 5 + overhead),
        ] {
            assert_eq!(ciphertext_size(clear, combo), cipher, "{combo:?} {clear}");
            assert_eq!(
                cleartext_size(cipher, combo),
                Ok(clear),
                "{combo:?} {cipher}"
            );
            assert_eq!(file_size(clear, combo), cipher + header);
            assert_eq!(cleartext_file_size(cipher + header, combo), Ok(clear));
        }
        // A last chunk shorter than nonce and tag is no ciphertext; exactly
        // nonce and tag is the empty last chunk of CryptoLib's writer.
        for rest in 1..overhead {
            assert_eq!(
                cleartext_size(chunk + rest, combo),
                Err(FormatError::InvalidSize)
            );
        }
        assert_eq!(cleartext_size(chunk + overhead, combo), Ok(payload));

        // Round trips at the boundaries: written length = formula.
        let cryptor = cryptor(combo);
        for clear in [0usize, 1, 32_767, 32_768, 32_769, 65_536, 65_541] {
            let data: Vec<u8> = (0..clear).map(|index| (index % 251) as u8).collect();
            for piece in [1000, 32_768, 70_000] {
                let file = write_all(&data, &cryptor, piece);
                assert_eq!(file.len() as u64, file_size(clear as u64, combo));
                assert_eq!(read_all(&file, &cryptor).unwrap(), data);
            }
            let in_memory = encrypt_bytes(&cryptor, &data).unwrap();
            assert_eq!(in_memory.len() as u64, file_size(clear as u64, combo));
            assert_eq!(
                &decrypt_bytes(&cryptor, &in_memory, clear).unwrap()[..],
                &data[..]
            );
            if clear > 0 {
                assert_eq!(
                    decrypt_bytes(&cryptor, &in_memory, clear - 1).unwrap_err(),
                    FormatError::TooLarge("Entschlüsselter Inhalt")
                );
            }
        }

        // A damaged second chunk: the first chunk is returned, then only
        // errors, never bytes of the damaged chunk.
        let data = vec![7u8; 2 * CHUNK_PAYLOAD + 10];
        let file = write_all(&data, &cryptor, 4096);
        let mut damaged = file.clone();
        damaged[(header + chunk) as usize + 20] ^= 0x01;
        let mut reader = DecryptingReader::new(&damaged[..], cryptor.clone()).unwrap();
        let mut first = vec![0u8; CHUNK_PAYLOAD];
        reader.read_exact(&mut first).unwrap();
        assert_eq!(first, data[..CHUNK_PAYLOAD]);
        let mut buffer = [0u8; 64];
        assert_eq!(
            reader.read(&mut buffer).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        assert!(reader.read(&mut buffer).is_err());
        // Swapped chunks fail too (the chunk number is authenticated).
        let mut swapped = file.clone();
        let (start, size) = (header as usize, chunk as usize);
        let first_chunk = swapped[start..start + size].to_vec();
        let second_chunk = swapped[start + size..start + 2 * size].to_vec();
        swapped[start..start + size].copy_from_slice(&second_chunk);
        swapped[start + size..start + 2 * size].copy_from_slice(&first_chunk);
        assert!(read_all(&swapped, &cryptor).is_err());
        // Cut inside the last chunk's nonce and tag.
        let cut = &file[..file.len() - (10 + overhead as usize) + 3];
        assert!(read_all(cut, &cryptor).is_err());

        // Locking wipes the key: an open reader fails at its next chunk with
        // PermissionDenied, a new writer fails at once.
        let cell = MasterkeyCell::new(Masterkey::generate().unwrap());
        let locked = ContentCryptor::new(cell.clone(), combo);
        let file = write_all(&data, &locked, 8192);
        let mut reader = DecryptingReader::new(&file[..], locked.clone()).unwrap();
        reader.read_exact(&mut first).unwrap();
        cell.wipe();
        assert_eq!(
            reader.read(&mut buffer).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
        assert!(EncryptingWriter::new(Vec::new(), locked.clone()).is_err());
        assert_eq!(
            read_all(&file, &locked).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
    }
}
