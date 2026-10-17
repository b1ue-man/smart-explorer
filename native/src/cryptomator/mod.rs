//! Cryptomator vault format 8 (C1 of the vault plan): key file, vault
//! configuration, file names, directory IDs and file contents of both cipher
//! combinations, plus the bytes of a password change. Pure byte
//! transformations without I/O of their own and without platform code; the
//! vault layer (`vault/`, VLT-M2 and later) puts them on top of every se
//! backend.
//!
//! Key material lives only in `zeroize::Zeroizing` buffers. Every tag and MAC
//! check runs in constant time and comes before any plaintext is released;
//! every failure is a `FormatError`, never partially decrypted bytes.
#![allow(dead_code)] // consumers (vault layer, sessions, vault sync) land with VLT-M2 and later
#![allow(unused_imports)] // the format API below is re-exported for those consumers

mod core;

pub(crate) use self::core::{
    backup_file_name, backup_suffix, ciphertext_size, cleartext_file_size, cleartext_size,
    content_dir, decrypt_bytes, decrypt_name, dir_file_bytes, dir_id_hash, encrypt_bytes,
    encrypt_name, file_size, is_metadata_name, needs_shortening, new_dir_id, parse_dir_file,
    read_bounded, required_memory, shortened_name, CipherCombo, ContentCryptor, DecryptingReader,
    EncryptingWriter, FileHeader, FormatError, KeySource, Masterkey, MasterkeyCell, MasterkeyFile,
    UnverifiedConfig, VaultConfig, BACKUP_SUFFIX, CHUNK_PAYLOAD, CIPHER_SUFFIX, CONTENTS_FILE,
    DATA_DIR, DEFAULT_SHORTENING_THRESHOLD, DIR_FILE, DIR_FILE_READ_LIMIT, DIR_ID_BACKUP_FILE,
    INFLATED_NAME_FILE, INFLATED_NAME_READ_LIMIT, INUSE_SUFFIX, MASTERKEY_FILE,
    MASTERKEY_FILE_KEY_ID, MAX_SHORTENING_THRESHOLD, MAX_SYMLINK_TARGET_CHARS, METADATA_READ_LIMIT,
    MIN_SHORTENING_THRESHOLD, ROOT_DIR_ID, SHORTENED_SUFFIX, SYMLINK_FILE, VAULT_CONFIG_FILE,
};
