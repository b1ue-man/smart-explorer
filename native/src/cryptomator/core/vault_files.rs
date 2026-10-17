//! Fixed names, read limits and backup names of the vault tree (format 8,
//! CryptoFS 2.11.0 `Constants`, `BackupHelper`).
use std::io::{self, Read};

use sha2_011::{Digest, Sha256};

/// Vault configuration (JWT) in the vault root.
pub(crate) const VAULT_CONFIG_FILE: &str = "vault.cryptomator";
/// Key file in the vault root; the only key source se accepts (C1.4).
pub(crate) const MASTERKEY_FILE: &str = "masterkey.cryptomator";
/// `kid` of `vault.cryptomator` that names the key file.
pub(crate) const MASTERKEY_FILE_KEY_ID: &str = "masterkeyfile:masterkey.cryptomator";
/// Folder of all content directories.
pub(crate) const DATA_DIR: &str = "d";
/// Suffix of encrypted names.
pub(crate) const CIPHER_SUFFIX: &str = ".c9r";
/// Suffix of shortened names (folder with `name.c9s`).
pub(crate) const SHORTENED_SUFFIX: &str = ".c9s";
/// Suffix of "files in use" markers (Hub vaults, CryptoFS 2.10.0 and later).
pub(crate) const INUSE_SUFFIX: &str = ".c9u";
/// Node file of a folder: its directory ID, unencrypted UTF-8.
pub(crate) const DIR_FILE: &str = "dir.c9r";
/// Node file of a symbolic link: its target, encrypted like file content.
pub(crate) const SYMLINK_FILE: &str = "symlink.c9r";
/// Content of a file with a shortened name.
pub(crate) const CONTENTS_FILE: &str = "contents.c9r";
/// Full encrypted name of a shortened entry, unencrypted UTF-8.
pub(crate) const INFLATED_NAME_FILE: &str = "name.c9s";
/// Backup of a content directory's own ID, encrypted like file content.
pub(crate) const DIR_ID_BACKUP_FILE: &str = "dirid.c9r";
/// Suffix of key file and configuration backups.
pub(crate) const BACKUP_SUFFIX: &str = ".bkup";

/// Read limit of `masterkey.cryptomator` and `vault.cryptomator`: an own
/// limit of se (C1.3); valid files are smaller than 1 KiB, the limit only
/// stops unbounded reads of prepared files on foreign storage.
pub(crate) const METADATA_READ_LIMIT: usize = 64 * 1024;
/// Read limit of `dir.c9r` (CryptoFS reports larger files as broken).
pub(crate) const DIR_FILE_READ_LIMIT: usize = 1000;
/// Read limit of `name.c9s` (CryptoFS `MAX_FILENAME_BUFFER_SIZE`).
pub(crate) const INFLATED_NAME_READ_LIMIT: usize = 10 * 1024;
/// Longest symbolic link target in characters (CryptoFS `Symlinks`).
pub(crate) const MAX_SYMLINK_TARGET_CHARS: usize = 32_767;

/// Smallest `shorteningThreshold` of new vaults (Cryptomator Desktop).
pub(crate) const MIN_SHORTENING_THRESHOLD: u32 = 36;
/// Largest and default `shorteningThreshold` (Cryptomator Desktop).
pub(crate) const MAX_SHORTENING_THRESHOLD: u32 = 220;
pub(crate) const DEFAULT_SHORTENING_THRESHOLD: u32 = 220;

/// The suffix Cryptomator puts between a file name and `.bkup`: `.` and the
/// first 4 bytes of SHA-256 over the file's bytes as uppercase hex.
pub(crate) fn backup_suffix(file_bytes: &[u8]) -> String {
    let digest = Sha256::digest(file_bytes);
    format!(".{}", data_encoding::HEXUPPER.encode(&digest[..4]))
}

/// `<name>.<8 HEX>.bkup`, e.g. `masterkey.cryptomator.1A2B3C4D.bkup`.
pub(crate) fn backup_file_name(name: &str, file_bytes: &[u8]) -> String {
    format!("{name}{}{BACKUP_SUFFIX}", backup_suffix(file_bytes))
}

/// Vault metadata in the root (sync rule R1): the configuration, the key
/// file and their backups `<name>.<8 HEX>.bkup`.
pub(crate) fn is_metadata_name(name: &str) -> bool {
    if name == VAULT_CONFIG_FILE || name == MASTERKEY_FILE {
        return true;
    }
    let Some(stem) = name.strip_suffix(BACKUP_SUFFIX) else {
        return false;
    };
    [VAULT_CONFIG_FILE, MASTERKEY_FILE].iter().any(|base| {
        stem.strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|hex| {
                hex.len() == 8
                    && hex
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
            })
    })
}

/// Reads at most `limit` bytes; `None` when the source holds more. Never
/// reads more than `limit + 1` bytes.
pub(crate) fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    reader.take(cap).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Ok(None);
    }
    Ok(Some(bytes))
}
