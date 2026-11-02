//! Fixed vectors (plan C1.2): the CryptoLib 2.3.0 strings (headers and
//! chunks of both combinations, a whole GCM file, the key file), the 640
//! AES-SIV-512 cases of cryptomator/siv-mode, and the scrypt and read limits
//! of C1.3.
use std::io::Read;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::Value;
use zeroize::Zeroizing;

use super::cipher_combo::CipherCombo;
use super::content_chunks::{decrypt_chunk, encrypt_chunk_with_nonce};
use super::content_header::{decrypt_header, encrypt_header, FileHeader};
use super::content_stream::{ContentCryptor, DecryptingReader};
use super::kdf::{check_params, required_memory};
use super::keys::{Masterkey, MasterkeyCell};
use super::masterkey_file::MasterkeyFile;
use super::names::{siv_open, siv_seal};
use super::vault_config::UnverifiedConfig;
use super::FormatError;

const SIV_CASES: &str = include_str!("testdata/siv_mode_512.txt");
const WRAPPED_ZERO_KEY: &str = "mM+qoQ+o0qvPTiDAZYt+flaC3WbpNAx1sTXaUzxwpy0M9Ctj6Tih/Q==";
const VERSION_MAC_3: &str = "iUmRRHITuyJsJbVNqGNw+82YQ4A3Rma7j/y1v0DCVLA=";

fn b64(text: &str) -> Vec<u8> {
    STANDARD.decode(text).expect("Base64 eines Prüfvektors")
}

fn hex(text: &str) -> Vec<u8> {
    data_encoding::HEXLOWER
        .decode(text.as_bytes())
        .expect("Hex eines Prüffalls")
}

fn zero_key() -> Masterkey {
    Masterkey::from_raw(&[0u8; 64])
}

fn known_header(combo: CipherCombo, nonce: &[u8], content_key: [u8; 32]) -> FileHeader {
    FileHeader::from_parts(combo, nonce, Zeroizing::new(content_key)).expect("Dateikopf")
}

#[test]
fn vault_task_cryptolib_header_gcm_vector() {
    // v2/FileHeaderCryptorImplTest.testEncryption: masterkey, nonce and
    // content key all zero.
    let key = zero_key();
    let expected = b64("AAAAAAAAAAAAAAAAMVi/wrKflJEHTsXTuvOdGHJgA8o3pip00aL1jnUGNY7dSrEoTUrhey+maVG6P0F2RBmZR74SjU0=");
    let header = known_header(CipherCombo::SivGcm, &[0u8; 12], [0u8; 32]);
    assert_eq!(encrypt_header(&key, &header).unwrap(), expected);
    let decrypted = decrypt_header(&key, CipherCombo::SivGcm, &expected).unwrap();
    assert_eq!(decrypted.nonce(), &[0u8; 12]);
    assert_eq!(decrypted.content_key(), &[0u8; 32]);
    // Nonce, payload and tag are all authenticated.
    for at in [0, 20, 67] {
        let mut tampered = expected.clone();
        tampered[at] ^= 0x01;
        assert_eq!(
            decrypt_header(&key, CipherCombo::SivGcm, &tampered).unwrap_err(),
            FormatError::Authentication("Dateikopf")
        );
    }
    assert_eq!(
        decrypt_header(&key, CipherCombo::SivGcm, &expected[..67]).unwrap_err(),
        FormatError::InvalidHeader
    );
    let other = Masterkey::from_raw(&[1u8; 64]);
    assert_eq!(
        decrypt_header(&other, CipherCombo::SivGcm, &expected).unwrap_err(),
        FormatError::Authentication("Dateikopf")
    );
}

#[test]
fn vault_task_cryptolib_header_ctrmac_vector() {
    // v1/FileHeaderCryptorImplTest.testEncryption: masterkey, nonce
    // (16 bytes) and content key all zero.
    let key = zero_key();
    let expected = b64("AAAAAAAAAAAAAAAAAAAAACNqP4ddv3Z2rUiiFJKEIIdTD4r7x0U2ualjtPHEy3OLzqdAPU1ga24VjC86+zlHN49BfMdzvHF3f9EE0LSnRLSsu6ps3IRcJg==");
    let header = known_header(CipherCombo::SivCtrMac, &[0u8; 16], [0u8; 32]);
    assert_eq!(encrypt_header(&key, &header).unwrap(), expected);
    let decrypted = decrypt_header(&key, CipherCombo::SivCtrMac, &expected).unwrap();
    assert_eq!(decrypted.nonce(), &[0u8; 16]);
    assert_eq!(decrypted.content_key(), &[0u8; 32]);
    // The MAC covers nonce and encrypted payload and is checked first.
    for at in [0, 30, 87] {
        let mut tampered = expected.clone();
        tampered[at] ^= 0x01;
        assert_eq!(
            decrypt_header(&key, CipherCombo::SivCtrMac, &tampered).unwrap_err(),
            FormatError::Authentication("Dateikopf")
        );
    }
    let mut other_mac = [0u8; 64];
    other_mac[32..].fill(1);
    assert_eq!(
        decrypt_header(
            &Masterkey::from_raw(&other_mac),
            CipherCombo::SivCtrMac,
            &expected
        )
        .unwrap_err(),
        FormatError::Authentication("Dateikopf")
    );
}

#[test]
fn vault_task_cryptolib_chunk_gcm_vector() {
    // v2/FileContentCryptorImplTest: content key and header nonce zero,
    // chunk 0; encryption with chunk nonce 12 × 0x33.
    let key = zero_key();
    let header = known_header(CipherCombo::SivGcm, &[0u8; 12], [0u8; 32]);
    let expected = b64("MzMzMzMzMzMzMzMzbYvL7CusRmzk70Kn1QxFA5WQg/hgKeba4bln");
    let encrypted =
        encrypt_chunk_with_nonce(&key, &header, 0, b"hello world", &[0x33; 12]).unwrap();
    assert_eq!(encrypted, expected);
    assert_eq!(
        &decrypt_chunk(&key, &header, 0, &expected).unwrap()[..],
        b"hello world"
    );
    // testChunkDecryption: a chunk written with nonce 12 × 0x55.
    let foreign = b64("VVVVVVVVVVVVVVVVnHVdh+EbedvPeiCwCdaTYpzn1CXQjhSh7PHv");
    assert_eq!(
        &decrypt_chunk(&key, &header, 0, &foreign).unwrap()[..],
        b"hello world"
    );
    // Chunk number and header nonce are associated data.
    assert_eq!(
        decrypt_chunk(&key, &header, 1, &expected).unwrap_err(),
        FormatError::Authentication("Block")
    );
    let other_header = known_header(CipherCombo::SivGcm, &[1u8; 12], [0u8; 32]);
    assert_eq!(
        decrypt_chunk(&key, &other_header, 0, &expected).unwrap_err(),
        FormatError::Authentication("Block")
    );
    let mut tampered = expected.clone();
    tampered[14] ^= 0x80;
    assert_eq!(
        decrypt_chunk(&key, &header, 0, &tampered).unwrap_err(),
        FormatError::Authentication("Block")
    );
    assert_eq!(
        decrypt_chunk(&key, &header, 0, &expected[..27]).unwrap_err(),
        FormatError::InvalidChunk
    );
}

#[test]
fn vault_task_cryptolib_chunk_ctrmac_vector() {
    // v1/FileContentCryptorImplTest.testChunkEncryption: masterkey, header
    // nonce, content key and chunk nonce all zero, chunk 0.
    let key = zero_key();
    let header = known_header(CipherCombo::SivCtrMac, &[0u8; 16], [0u8; 32]);
    let expected =
        b64("AAAAAAAAAAAAAAAAAAAAALTwrBTNYP7m3yTGKlhka9WPvX1Lpn5EYfVxlyX1ISgRXtdRnivM7r6F3Og=");
    let encrypted = encrypt_chunk_with_nonce(&key, &header, 0, b"hello world", &[0u8; 16]).unwrap();
    assert_eq!(encrypted, expected);
    assert_eq!(
        &decrypt_chunk(&key, &header, 0, &expected).unwrap()[..],
        b"hello world"
    );
    // The chunk MAC uses the MAC masterkey over header nonce ‖ chunk number
    // ‖ chunk nonce ‖ ciphertext.
    assert_eq!(
        decrypt_chunk(&key, &header, 1, &expected).unwrap_err(),
        FormatError::Authentication("Block")
    );
    let mut other_mac = [0u8; 64];
    other_mac[32..].fill(1);
    assert_eq!(
        decrypt_chunk(&Masterkey::from_raw(&other_mac), &header, 0, &expected).unwrap_err(),
        FormatError::Authentication("Block")
    );
    let mut tampered = expected.clone();
    tampered[20] ^= 0x01;
    assert_eq!(
        decrypt_chunk(&key, &header, 0, &tampered).unwrap_err(),
        FormatError::Authentication("Block")
    );
}

#[test]
fn vault_task_cryptolib_file_gcm_vector() {
    // v2/FileContentCryptorImplTest.testFileEncryption: masterkey zero; the
    // three random draws give header nonce 12 × 0x55, content key 32 × 0x77
    // and chunk nonce 12 × 0x55.
    let expected = b64("VVVVVVVVVVVVVVVVC+/OFHHE8UvKYTOPlrMO5rCRLAI7/zk8Hjoisja03+yi9ugeeMz1evZhxDExrawl93vf9DKQPx5VVVVVVVVVVVVVVVVSxe6Nf7RO8orsVTzHAmXlNSy1oJpDrg9coV0=");
    let key = zero_key();
    let header = known_header(CipherCombo::SivGcm, &[0x55; 12], [0x77; 32]);
    let mut file = encrypt_header(&key, &header).unwrap();
    file.extend(encrypt_chunk_with_nonce(&key, &header, 0, b"hello world", &[0x55; 12]).unwrap());
    assert_eq!(file, expected);
    // The CryptoLib file read through the stream reader of se.
    let cryptor = ContentCryptor::new(MasterkeyCell::new(zero_key()), CipherCombo::SivGcm);
    let mut reader = DecryptingReader::new(&expected[..], cryptor).unwrap();
    let mut plain = Vec::new();
    reader.read_to_end(&mut plain).unwrap();
    assert_eq!(plain, b"hello world");
}

#[test]
fn vault_task_cryptolib_masterkey_file_vector() {
    // common/MasterkeyFileAccessTest: password "asd", salt 8 × 0, N = 2,
    // r = 8, empty pepper, masterkey zero, version 3.
    let key = zero_key();
    let bytes = MasterkeyFile::create_with(&key, "asd", &[0u8; 8], 2, 8, 3, None).unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["primaryMasterKey"], WRAPPED_ZERO_KEY);
    assert_eq!(json["hmacMasterKey"], WRAPPED_ZERO_KEY);
    assert_eq!(json["versionMac"], VERSION_MAC_3);
    assert_eq!(json["version"], 3);
    assert_eq!(json["scryptSalt"], "AAAAAAAAAAA=");
    assert_eq!(json["scryptCostParam"], 2);
    assert_eq!(json["scryptBlockSize"], 8);

    let file = MasterkeyFile::parse(&bytes).unwrap();
    assert_eq!(file.version(), 3);
    assert_eq!(&file.unlock("asd", None).unwrap().raw()[..], &[0u8; 64][..]);
    assert_eq!(
        file.unlock("asD", None).unwrap_err(),
        FormatError::WrongPassword
    );

    // A file as CryptoLib writes it, with an unknown extra field.
    let foreign = format!(
        r#"{{"version": 3, "scryptSalt": "AAAAAAAAAAA=", "scryptCostParam": 2,
            "scryptBlockSize": 8, "primaryMasterKey": "{WRAPPED_ZERO_KEY}",
            "hmacMasterKey": "{WRAPPED_ZERO_KEY}", "versionMac": "{VERSION_MAC_3}", "x": 1}}"#
    );
    let foreign = MasterkeyFile::parse(foreign.as_bytes()).unwrap();
    assert_eq!(
        &foreign.unlock("asd", None).unwrap().raw()[..],
        &[0u8; 64][..]
    );
    for broken in [
        r#"{"version": 3}"#,
        r#"[1, 2]"#,
        r#"{"version": 3, "scryptSalt": "AAAAAAAAAAA=", "scryptCostParam": 1, "scryptBlockSize": 8,
            "primaryMasterKey": "AA==", "hmacMasterKey": "AA==", "versionMac": "AA=="}"#,
    ] {
        assert!(matches!(
            MasterkeyFile::parse(broken.as_bytes()),
            Err(FormatError::InvalidMasterkeyFile(_))
        ));
    }

    // Password change bytes: new salt, same N, r, version, versionMac and
    // masterkey; the old password no longer opens the new file.
    let changed = file.rewrap("asd", "neu", None).unwrap();
    let changed_json: Value = serde_json::from_slice(&changed).unwrap();
    assert_eq!(changed_json["version"], 3);
    assert_eq!(changed_json["versionMac"], VERSION_MAC_3);
    assert_eq!(changed_json["scryptCostParam"], 2);
    assert_eq!(changed_json["scryptBlockSize"], 8);
    let new_salt = changed_json["scryptSalt"].as_str().unwrap();
    assert_eq!(b64(new_salt).len(), 8);
    assert_ne!(new_salt, "AAAAAAAAAAA=");
    let changed = MasterkeyFile::parse(&changed).unwrap();
    assert_eq!(
        &changed.unlock("neu", None).unwrap().raw()[..],
        &[0u8; 64][..]
    );
    assert_eq!(
        changed.unlock("asd", None).unwrap_err(),
        FormatError::WrongPassword
    );
    assert_eq!(
        file.rewrap("falsch", "neu", None).unwrap_err(),
        FormatError::WrongPassword
    );

    // Passwords are normalized to NFC before scrypt.
    let generated = Masterkey::generate().unwrap();
    let nfc = "P\u{e4}sswort";
    let nfd = "Pa\u{308}sswort";
    let bytes = MasterkeyFile::create_with(&generated, nfc, &[9u8; 8], 4, 8, 999, None).unwrap();
    let unlocked = MasterkeyFile::parse(&bytes)
        .unwrap()
        .unlock(nfd, None)
        .unwrap();
    assert_eq!(&unlocked.raw()[..], &generated.raw()[..]);
}

#[test]
fn vault_task_siv_mode_512_cases() {
    let mut cases = 0usize;
    let mut empty_items = 0usize;
    for line in SIV_CASES
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<&str> = line.split(';').collect();
        let (ctr_key, mac_key, plaintext) = (hex(fields[0]), hex(fields[1]), hex(fields[2]));
        let count: usize = fields[3].parse().unwrap();
        assert!(count <= 1, "Fall {cases}: {count} Zusatzdatenfelder");
        assert_eq!(fields.len(), 5 + count, "Fall {cases}");
        let associated = (count == 1).then(|| hex(fields[4]));
        let expected = hex(fields[4 + count]);
        let mut key = mac_key;
        key.extend_from_slice(&ctr_key);
        assert_eq!(key.len(), 64);
        let sealed = siv_seal(&key, associated.as_deref(), &plaintext).unwrap();
        assert_eq!(sealed, expected, "Fall {cases}");
        let opened = siv_open(&key, associated.as_deref(), &expected, "Fall").unwrap();
        assert_eq!(opened, plaintext, "Fall {cases}");
        let mut tampered = expected.clone();
        tampered[0] ^= 0x01;
        assert!(siv_open(&key, associated.as_deref(), &tampered, "Fall").is_err());
        if associated.as_ref().is_some_and(|item| item.is_empty()) {
            empty_items += 1;
        }
        cases += 1;
    }
    assert_eq!(cases, 640);
    assert_eq!(empty_items, 160);
}

#[test]
fn vault_task_scrypt_bounds_reference_formula() {
    // 128 · r · (N + 3) ≤ 1 GiB + 3 072 bytes (CryptoLib 2.3.0).
    assert_eq!(required_memory(1 << 20, 8), 1_073_744_896);
    assert_eq!(required_memory(1 << 17, 64), 1_073_766_400);
    assert_eq!(check_params(1 << 20, 8, None), Ok((20, 8)));
    assert_eq!(check_params(1 << 15, 8, None), Ok((15, 8)));
    for (cost, block_size) in [
        (1 << 17, 64),
        (3, 8),
        (1, 8),
        (0, 8),
        (-4, 8),
        (1 << 21, 1),
        (1 << 10, 0),
        (1 << 10, 65),
    ] {
        assert!(
            matches!(
                check_params(cost, block_size, None),
                Err(FormatError::KdfLimits(_))
            ),
            "N = {cost}, r = {block_size}"
        );
    }
    // The memory this process may use (C1.3 (c)).
    let needed = required_memory(1 << 15, 8);
    assert_eq!(check_params(1 << 15, 8, Some(needed)), Ok((15, 8)));
    assert_eq!(
        check_params(1 << 15, 8, Some(needed - 1)),
        Err(FormatError::KdfMemory {
            needed,
            available: needed - 1
        })
    );
    assert_eq!(
        check_params(1 << 20, 8, Some(512 * 1024 * 1024))
            .unwrap_err()
            .to_string(),
        "Schlüsselableitung braucht 1025 MiB, verfügbar 512 MiB"
    );
    // Refused before scrypt allocates: N = 2^30 would need 1 TiB.
    let hostile = format!(
        r#"{{"version": 999, "scryptSalt": "AAAAAAAAAAA=", "scryptCostParam": {},
            "scryptBlockSize": 8, "primaryMasterKey": "{WRAPPED_ZERO_KEY}",
            "hmacMasterKey": "{WRAPPED_ZERO_KEY}", "versionMac": "{VERSION_MAC_3}"}}"#,
        1u64 << 30
    );
    let hostile = MasterkeyFile::parse(hostile.as_bytes()).unwrap();
    assert!(matches!(
        hostile.unlock("asd", None),
        Err(FormatError::KdfLimits(_))
    ));
}

/// A source that never ends and counts what it served.
struct Endless {
    served: usize,
}

impl Read for Endless {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        buffer.fill(b' ');
        self.served += buffer.len();
        Ok(buffer.len())
    }
}

#[test]
fn vault_task_masterkey_file_over_64k_rejected() {
    let limit = 64 * 1024;
    assert_eq!(
        MasterkeyFile::parse(&vec![b' '; limit + 1]).unwrap_err(),
        FormatError::MasterkeyFileTooLarge
    );
    assert_eq!(
        FormatError::MasterkeyFileTooLarge.to_string(),
        "Schlüsseldatei größer als 64 KiB – kein gültiger Tresor"
    );
    // Reading stops after the limit plus one byte.
    let mut source = Endless { served: 0 };
    assert_eq!(
        MasterkeyFile::read_from(&mut source).unwrap_err(),
        FormatError::MasterkeyFileTooLarge
    );
    assert!(source.served <= limit + 1, "{} Byte gelesen", source.served);
    // Exactly 64 KiB are read and parsed (here: no JSON).
    assert!(matches!(
        MasterkeyFile::parse(&vec![b' '; limit]),
        Err(FormatError::InvalidMasterkeyFile(_))
    ));
    // A valid file padded to the limit still opens.
    let bytes = MasterkeyFile::create_with(&zero_key(), "asd", &[0u8; 8], 2, 8, 3, None).unwrap();
    let mut padded = bytes.clone();
    padded.resize(limit, b' ');
    let file = MasterkeyFile::read_from(&padded[..]).unwrap();
    assert_eq!(&file.unlock("asd", None).unwrap().raw()[..], &[0u8; 64][..]);
    // The vault configuration has the same bound.
    let mut source = Endless { served: 0 };
    assert_eq!(
        UnverifiedConfig::read_from(&mut source).unwrap_err(),
        FormatError::ConfigTooLarge
    );
    assert!(source.served <= limit + 1);
}
