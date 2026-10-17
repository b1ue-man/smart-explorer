//! File names (format 8): the NFC-normalized cleartext name, AES-SIV with
//! the parent's directory ID as associated data, Base64url with padding
//! (Guava `base64Url()`), suffix `.c9r`. A name longer than the vault's
//! `shorteningThreshold` is stored as `BASE64URL(SHA-1(name)).c9s`.
use base64::engine::general_purpose::URL_SAFE;
use base64::Engine as _;
use sha1_011::{Digest, Sha1};

use super::keys::{Masterkey, MASTERKEY_LEN};
use super::vault_files::{CIPHER_SUFFIX, SHORTENED_SUFFIX};
use super::FormatError;

/// Length of the synthetic IV in front of every SIV ciphertext.
const SIV_TAG_LEN: usize = 16;

/// AES-SIV (RFC 5297, CMAC-AES-256 + CTR-AES-256) with the 64-byte key
/// MAC key ‖ encryption key and zero or one associated-data item (the two
/// call forms of Cryptomator: directory hash, name with parent ID). Output:
/// synthetic IV ‖ ciphertext.
pub(crate) fn siv_seal(
    siv_key: &[u8],
    associated: Option<&[u8]>,
    plaintext: &[u8],
) -> Result<Vec<u8>, FormatError> {
    use aes_siv::{siv::Aes256Siv, KeyInit};
    if siv_key.len() != MASTERKEY_LEN {
        return Err(FormatError::Crypto("AES-SIV"));
    }
    let mut siv = Aes256Siv::new_from_slice(siv_key).map_err(|_| FormatError::Crypto("AES-SIV"))?;
    let sealed = match associated {
        Some(data) => siv.encrypt([data], plaintext),
        None => siv.encrypt(std::iter::empty::<&[u8]>(), plaintext),
    };
    sealed.map_err(|_| FormatError::Crypto("AES-SIV"))
}

/// Checks and decrypts an AES-SIV ciphertext (synthetic IV ‖ ciphertext);
/// the IV is compared in constant time and nothing is returned on mismatch.
pub(crate) fn siv_open(
    siv_key: &[u8],
    associated: Option<&[u8]>,
    sealed: &[u8],
    what: &'static str,
) -> Result<Vec<u8>, FormatError> {
    use aes_siv::{siv::Aes256Siv, KeyInit};
    if siv_key.len() != MASTERKEY_LEN {
        return Err(FormatError::Crypto("AES-SIV"));
    }
    if sealed.len() < SIV_TAG_LEN {
        return Err(FormatError::Authentication(what));
    }
    let mut siv = Aes256Siv::new_from_slice(siv_key).map_err(|_| FormatError::Crypto("AES-SIV"))?;
    let opened = match associated {
        Some(data) => siv.decrypt([data], sealed),
        None => siv.decrypt(std::iter::empty::<&[u8]>(), sealed),
    };
    opened.map_err(|_| FormatError::Authentication(what))
}

/// The encrypted name `<Base64url>.c9r` of `cleartext` in the folder with
/// the directory ID `parent_dir_id`. The name is normalized to NFC first,
/// as CryptoFS does, so NFC and NFD spellings give the same entry.
pub(crate) fn encrypt_name(
    key: &Masterkey,
    cleartext: &str,
    parent_dir_id: &str,
) -> Result<String, FormatError> {
    if cleartext.is_empty() {
        return Err(FormatError::InvalidName);
    }
    let normalized = icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(cleartext);
    let sealed = siv_seal(
        &key.siv_key()[..],
        Some(parent_dir_id.as_bytes()),
        normalized.as_bytes(),
    )?;
    Ok(format!("{}{CIPHER_SUFFIX}", URL_SAFE.encode(sealed)))
}

/// The cleartext of an encrypted name `<Base64url>.c9r` in the folder with
/// the directory ID `parent_dir_id`. A name that is no plain encrypted name
/// (conflict copies with a suffix, foreign files) is `InvalidName`; a name
/// that does not authenticate is `Authentication`.
pub(crate) fn decrypt_name(
    key: &Masterkey,
    ciphertext_name: &str,
    parent_dir_id: &str,
) -> Result<String, FormatError> {
    let encoded = ciphertext_name
        .strip_suffix(CIPHER_SUFFIX)
        .ok_or(FormatError::InvalidName)?;
    let sealed = URL_SAFE
        .decode(encoded.as_bytes())
        .map_err(|_| FormatError::InvalidName)?;
    if sealed.len() <= SIV_TAG_LEN {
        return Err(FormatError::InvalidName);
    }
    let plain = siv_open(
        &key.siv_key()[..],
        Some(parent_dir_id.as_bytes()),
        &sealed,
        "Dateiname",
    )?;
    String::from_utf8(plain).map_err(|_| FormatError::InvalidName)
}

/// The shortened name of an encrypted name that exceeds the threshold:
/// `BASE64URL(SHA-1(<full name with .c9r>)).c9s` (32 characters).
pub(crate) fn shortened_name(ciphertext_name: &str) -> String {
    let digest = Sha1::digest(ciphertext_name.as_bytes());
    format!("{}{SHORTENED_SUFFIX}", URL_SAFE.encode(digest))
}

/// Whether an encrypted name (with `.c9r`) is stored shortened: longer than
/// the vault's `shorteningThreshold` (CryptoFS `CryptoPathMapper`).
pub(crate) fn needs_shortening(ciphertext_name: &str, shortening_threshold: u32) -> bool {
    u64::try_from(ciphertext_name.len()).unwrap_or(u64::MAX) > u64::from(shortening_threshold)
}
