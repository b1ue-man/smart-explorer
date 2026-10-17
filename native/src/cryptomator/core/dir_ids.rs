//! Directory IDs and content directories (format 8): the root has the empty
//! ID, every other folder a random ID (UUIDv4 text, at most 36 ASCII
//! characters) stored unencrypted in its node file `dir.c9r`. Its content
//! lies flat under `d/` at `d/<2>/<30>`, the two parts of
//! `BASE32(SHA-1(AES-SIV(ID)))` (no associated data).
use sha1_011::{Digest, Sha1};

use super::keys::{fill_random, Masterkey};
use super::names::siv_seal;
use super::vault_files::{DATA_DIR, DIR_FILE_READ_LIMIT};
use super::FormatError;

/// Directory ID of the vault root.
pub(crate) const ROOT_DIR_ID: &str = "";
/// Length of the IDs se creates (UUIDv4 text).
pub(crate) const NEW_DIR_ID_LEN: usize = 36;

/// A new directory ID: 16 random bytes as UUIDv4 text (lowercase, with
/// dashes), as Java `UUID.randomUUID()` writes it. No dependency on the
/// `uuid` crate (C1.1).
pub(crate) fn new_dir_id() -> Result<String, FormatError> {
    let mut bytes = [0u8; 16];
    fill_random(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = data_encoding::HEXLOWER.encode(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// `BASE32(SHA-1(AES-SIV(dir_id)))`: 32 characters of `A`–`Z`, `2`–`7`.
pub(crate) fn dir_id_hash(key: &Masterkey, dir_id: &str) -> Result<String, FormatError> {
    let sealed = siv_seal(&key.siv_key()[..], None, dir_id.as_bytes())?;
    let digest = Sha1::digest(&sealed);
    Ok(data_encoding::BASE32.encode(&digest))
}

/// The content directory of `dir_id` relative to the vault root:
/// `d/<first 2 hash characters>/<other 30>`.
pub(crate) fn content_dir(key: &Masterkey, dir_id: &str) -> Result<String, FormatError> {
    let hash = dir_id_hash(key, dir_id)?;
    if hash.len() != 32 {
        return Err(FormatError::Crypto("Verzeichnis-Hash"));
    }
    Ok(format!("{DATA_DIR}/{}/{}", &hash[..2], &hash[2..]))
}

/// The bytes of a node file `dir.c9r` for `dir_id`.
pub(crate) fn dir_file_bytes(dir_id: &str) -> Vec<u8> {
    dir_id.as_bytes().to_vec()
}

/// The directory ID stored in a node file `dir.c9r`: non-empty UTF-8 of at
/// most `DIR_FILE_READ_LIMIT` bytes (CryptoFS reads it the same way; IDs
/// that se creates are 36 characters).
pub(crate) fn parse_dir_file(bytes: &[u8]) -> Result<String, FormatError> {
    if bytes.is_empty() || bytes.len() > DIR_FILE_READ_LIMIT {
        return Err(FormatError::InvalidDirId);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| FormatError::InvalidDirId)
}
