//! Key-encryption key from the password (C1.3, C3.1): NFC-normalized UTF-8
//! password, scrypt with the key file's salt (empty pepper, p = 1) and
//! 32 bytes of output; AES Key Wrap (RFC 3394) of the two masterkey halves.
//!
//! The scrypt limits apply before scrypt allocates anything: N is a power of
//! two greater than 1 and at most 2^20, r is 1 to 64, the memory
//! `128 · r · (N + 3)` stays within 1 GiB + 3 072 bytes (CryptoLib 2.3.0
//! `MasterkeyFileValidator`), and it fits the memory this process may use
//! (`available`, measured by the caller: desktop `MemAvailable`/
//! `GlobalMemoryStatusEx`, Android `availMem − threshold`).
use std::borrow::Cow;

use aes_kw::{KeyInit, KwAes256};
use zeroize::Zeroizing;

use super::keys::SUBKEY_LEN;
use super::FormatError;

/// Largest scrypt cost parameter N (CryptoLib 2.3.0).
pub(crate) const MAX_COST: i64 = 1 << 20;
/// Largest scrypt block size r (CryptoLib 2.3.0).
pub(crate) const MAX_BLOCK_SIZE: i64 = 64;
/// Largest scrypt working memory: 1 GiB + 3 072 bytes (CryptoLib 2.3.0).
pub(crate) const MAX_MEMORY: u64 = (1 << 30) + 3072;
/// Length of an AES-KW-wrapped 256-bit key.
pub(crate) const WRAPPED_LEN: usize = SUBKEY_LEN + 8;

/// scrypt working memory in bytes: `128 · r · (N + 3)` (saturating).
pub(crate) fn required_memory(cost: u64, block_size: u64) -> u64 {
    128u64
        .saturating_mul(block_size)
        .saturating_mul(cost.saturating_add(3))
}

/// Checks N and r against C1.3 and the available memory; returns log₂ N
/// and r for `scrypt::Params`.
pub(crate) fn check_params(
    cost: i64,
    block_size: i64,
    available: Option<u64>,
) -> Result<(u8, u32), FormatError> {
    if cost <= 1 || cost.count_ones() != 1 {
        return Err(FormatError::KdfLimits(
            "Kostenparameter N ist keine Zweierpotenz größer 1".into(),
        ));
    }
    if cost > MAX_COST {
        return Err(FormatError::KdfLimits(
            "Kostenparameter N größer als 2^20".into(),
        ));
    }
    if !(1..=MAX_BLOCK_SIZE).contains(&block_size) {
        return Err(FormatError::KdfLimits(
            "Blockgröße r außerhalb von 1 bis 64".into(),
        ));
    }
    // Both are positive and bounded above, so the conversions are exact.
    let needed = required_memory(cost as u64, block_size as u64);
    if needed > MAX_MEMORY {
        return Err(FormatError::KdfLimits(
            "Speicherbedarf über 1 GiB + 3 072 Byte".into(),
        ));
    }
    if let Some(available) = available {
        if needed > available {
            return Err(FormatError::KdfMemory { needed, available });
        }
    }
    Ok((cost.trailing_zeros() as u8, block_size as u32))
}

/// The password as scrypt reads it: NFC-normalized UTF-8 (Cryptomator
/// normalizes passwords to NFC; CryptoLib encodes UTF-8 without normalizing).
pub(crate) fn password_bytes(password: &str) -> Zeroizing<Vec<u8>> {
    match icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(password) {
        Cow::Borrowed(text) => Zeroizing::new(text.as_bytes().to_vec()),
        Cow::Owned(text) => Zeroizing::new(text.into_bytes()),
    }
}

/// scrypt(password, salt, N, r, p = 1) with 32 bytes of output, after the
/// limits of `check_params`.
pub(crate) fn derive_kek(
    password: &str,
    salt: &[u8],
    cost: i64,
    block_size: i64,
    available: Option<u64>,
) -> Result<Zeroizing<[u8; SUBKEY_LEN]>, FormatError> {
    let (log_n, r) = check_params(cost, block_size, available)?;
    let params = scrypt::Params::new(log_n, r, 1)
        .map_err(|_| FormatError::KdfLimits("Parameter von scrypt abgelehnt".into()))?;
    let password = password_bytes(password);
    let mut kek = Zeroizing::new([0u8; SUBKEY_LEN]);
    scrypt::scrypt(&password, salt, &params, &mut kek[..])
        .map_err(|_| FormatError::Crypto("scrypt"))?;
    Ok(kek)
}

/// AES Key Wrap (RFC 3394) of one masterkey half.
pub(crate) fn wrap_key(
    kek: &[u8; SUBKEY_LEN],
    key: &[u8; SUBKEY_LEN],
) -> Result<[u8; WRAPPED_LEN], FormatError> {
    let wrapper = KwAes256::new_from_slice(kek).map_err(|_| FormatError::Crypto("AES-KW"))?;
    let mut wrapped = [0u8; WRAPPED_LEN];
    wrapper
        .wrap_key(key, &mut wrapped)
        .map_err(|_| FormatError::Crypto("AES-KW"))?;
    Ok(wrapped)
}

/// Unwraps one masterkey half. A failed integrity check means a wrong
/// password (or a damaged key file) and yields no key bytes.
pub(crate) fn unwrap_key(
    kek: &[u8; SUBKEY_LEN],
    wrapped: &[u8; WRAPPED_LEN],
) -> Result<Zeroizing<[u8; SUBKEY_LEN]>, FormatError> {
    let wrapper = KwAes256::new_from_slice(kek).map_err(|_| FormatError::Crypto("AES-KW"))?;
    let mut key = Zeroizing::new([0u8; SUBKEY_LEN]);
    wrapper
        .unwrap_key(wrapped, &mut key[..])
        .map_err(|_| FormatError::WrongPassword)?;
    Ok(key)
}
