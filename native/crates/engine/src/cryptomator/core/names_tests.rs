//! Encrypted names, directory IDs and content directories, shortened names.
//! Fixed vectors for names do not exist in CryptoLib or CryptoFS (plan K2);
//! AES-SIV itself is covered by the 640 siv-mode cases, the exact mapping by
//! the two-sided CryptoFS check of stage `vault_interop`.
use super::dir_ids::{
    content_dir, dir_file_bytes, dir_id_hash, new_dir_id, parse_dir_file, ROOT_DIR_ID,
};
use super::keys::Masterkey;
use super::names::{decrypt_name, encrypt_name, needs_shortening, shortened_name};
use super::FormatError;

const PARENT: &str = "ea3282b3-3847-499b-82fd-a3723857a225";

fn key() -> Masterkey {
    let raw: [u8; 64] = std::array::from_fn(|index| (index as u8).wrapping_mul(3));
    Masterkey::from_raw(&raw)
}

fn is_base64url_padded(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='))
}

#[test]
fn vault_task_name_roundtrip_nfc_nfd() {
    let key = key();
    let nfc = "Gr\u{fc}\u{df}e \u{e4}.txt";
    let nfd = "Gru\u{308}\u{df}e a\u{308}.txt";
    assert_ne!(nfc, nfd);
    // Normalized to NFC before encryption, as CryptoFS does: both
    // spellings name the same entry.
    let from_nfc = encrypt_name(&key, nfc, PARENT).unwrap();
    let from_nfd = encrypt_name(&key, nfd, PARENT).unwrap();
    assert_eq!(from_nfc, from_nfd);
    let encoded = from_nfc.strip_suffix(".c9r").unwrap();
    assert!(is_base64url_padded(encoded), "{from_nfc}");
    assert_eq!(decrypt_name(&key, &from_nfc, PARENT).unwrap(), nfc);
    // Deterministic, and bound to the parent's directory ID.
    assert_eq!(encrypt_name(&key, nfc, PARENT).unwrap(), from_nfc);
    let in_root = encrypt_name(&key, nfc, ROOT_DIR_ID).unwrap();
    assert_ne!(in_root, from_nfc);
    assert_eq!(decrypt_name(&key, &in_root, ROOT_DIR_ID).unwrap(), nfc);
    assert_eq!(
        decrypt_name(&key, &from_nfc, ROOT_DIR_ID).unwrap_err(),
        FormatError::Authentication("Dateiname")
    );
    let other = Masterkey::from_raw(&[5u8; 64]);
    assert_eq!(
        decrypt_name(&other, &from_nfc, PARENT).unwrap_err(),
        FormatError::Authentication("Dateiname")
    );
    for name in [
        "a".to_string(),
        "Ordner mit Leerzeichen".to_string(),
        "\u{1F600}.png".to_string(),
        ".versteckt".to_string(),
        "x".repeat(255),
    ] {
        let encrypted = encrypt_name(&key, &name, PARENT).unwrap();
        assert_eq!(decrypt_name(&key, &encrypted, PARENT).unwrap(), name);
    }
    // Not an encrypted name: conflict copies with a suffix, foreign files,
    // an empty name, missing padding.
    let mut foreign = vec![
        "kein-chiffrat.txt".to_string(),
        "abc (1).c9r".to_string(),
        ".c9r".to_string(),
        format!("{encoded} (Konflikt).c9r"),
    ];
    if encoded.ends_with('=') {
        foreign.push(format!("{}.c9r", encoded.trim_end_matches('=')));
    }
    for name in &foreign {
        assert!(
            matches!(
                decrypt_name(&key, name, PARENT),
                Err(FormatError::InvalidName | FormatError::Authentication(_))
            ),
            "{name}"
        );
    }
    assert_eq!(
        encrypt_name(&key, "", PARENT).unwrap_err(),
        FormatError::InvalidName
    );
    let mut tampered = from_nfc.clone().into_bytes();
    tampered[3] = if tampered[3] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap();
    assert!(decrypt_name(&key, &tampered, PARENT).is_err());
}

#[test]
fn vault_task_dir_hash_shape() {
    let key = key();
    let id = new_dir_id().unwrap();
    let root = content_dir(&key, ROOT_DIR_ID).unwrap();
    let child = content_dir(&key, &id).unwrap();
    for dir in [&root, &child] {
        let parts: Vec<&str> = dir.split('/').collect();
        assert_eq!(parts.len(), 3, "{dir}");
        assert_eq!(parts[0], "d");
        assert_eq!(parts[1].len(), 2, "{dir}");
        assert_eq!(parts[2].len(), 30, "{dir}");
        assert!(parts[1..].iter().all(|part| part
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte))));
    }
    assert_ne!(root, child);
    assert_eq!(content_dir(&key, &id).unwrap(), child);
    let hash = dir_id_hash(&key, &id).unwrap();
    assert_eq!(hash.len(), 32);
    assert_eq!(child, format!("d/{}/{}", &hash[..2], &hash[2..]));
    let other = Masterkey::from_raw(&[5u8; 64]);
    assert_ne!(content_dir(&other, &id).unwrap(), child);

    // New IDs: UUIDv4 text as Java writes it.
    assert_eq!(id.len(), 36);
    for dash in [8, 13, 18, 23] {
        assert_eq!(&id[dash..dash + 1], "-", "{id}");
    }
    assert_eq!(&id[14..15], "4", "{id}");
    assert!("89ab".contains(&id[19..20]), "{id}");
    assert!(id
        .bytes()
        .all(|byte| byte == b'-' || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert_ne!(new_dir_id().unwrap(), id);

    // Node file `dir.c9r`: the ID unencrypted.
    assert_eq!(dir_file_bytes(&id), id.as_bytes());
    assert_eq!(parse_dir_file(&dir_file_bytes(&id)).unwrap(), id);
    assert_eq!(parse_dir_file(b"").unwrap_err(), FormatError::InvalidDirId);
    assert_eq!(
        parse_dir_file(&[b'a'; 1001]).unwrap_err(),
        FormatError::InvalidDirId
    );
    assert_eq!(
        parse_dir_file(&[0xFF, 0xFE]).unwrap_err(),
        FormatError::InvalidDirId
    );
}

#[test]
fn vault_task_shortened_name() {
    // BASE64URL(SHA-1("abc")) with padding, then ".c9s".
    assert_eq!(shortened_name("abc"), "qZk-NkcGgWq6PiVxeFDCbJzQ2J0=.c9s");
    let key = key();
    let long = "\u{e4}".repeat(150);
    let encrypted = encrypt_name(&key, &long, ROOT_DIR_ID).unwrap();
    assert!(needs_shortening(&encrypted, 220));
    let short = shortened_name(&encrypted);
    assert_eq!(short.len(), 32);
    assert!(short.ends_with(".c9s"));
    assert_eq!(shortened_name(&encrypted), short);
    // The full name (content of `name.c9s`) still decrypts.
    assert_eq!(decrypt_name(&key, &encrypted, ROOT_DIR_ID).unwrap(), long);
    // Threshold boundary: longer than the threshold is shortened.
    let at = "A".repeat(220);
    assert!(!needs_shortening(&at, 220));
    assert!(needs_shortening(&format!("{at}B"), 220));
    assert!(needs_shortening(&at, 36));
    let short_name = encrypt_name(&key, "a.txt", ROOT_DIR_ID).unwrap();
    assert!(!needs_shortening(&short_name, 36));
}
