//! The vault's 512-bit masterkey (encryption key ‖ MAC key, CryptoLib
//! `Masterkey`) and the cell that holds it while a vault is unlocked. Key
//! bytes live only in `Zeroizing` buffers and are wiped when dropped.
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use zeroize::Zeroizing;

use super::FormatError;

/// Length of each half of the masterkey and of every derived 256-bit key.
pub(crate) const SUBKEY_LEN: usize = 32;
/// Length of the raw masterkey (encryption key ‖ MAC key).
pub(crate) const MASTERKEY_LEN: usize = 2 * SUBKEY_LEN;

/// Encryption masterkey (AES-256: headers, CTR, SIV-CTR half) and MAC
/// masterkey (HMAC-SHA256, CMAC half of AES-SIV).
pub(crate) struct Masterkey {
    enc: Zeroizing<[u8; SUBKEY_LEN]>,
    mac: Zeroizing<[u8; SUBKEY_LEN]>,
}

impl Masterkey {
    /// A new random masterkey (64 bytes from the system generator).
    pub(crate) fn generate() -> Result<Self, FormatError> {
        let mut raw = Zeroizing::new([0u8; MASTERKEY_LEN]);
        fill_random(&mut raw[..])?;
        Ok(Self::from_raw(&raw))
    }

    /// The masterkey from its raw form encryption key ‖ MAC key.
    pub(crate) fn from_raw(raw: &[u8; MASTERKEY_LEN]) -> Self {
        let mut enc = Zeroizing::new([0u8; SUBKEY_LEN]);
        let mut mac = Zeroizing::new([0u8; SUBKEY_LEN]);
        enc.copy_from_slice(&raw[..SUBKEY_LEN]);
        mac.copy_from_slice(&raw[SUBKEY_LEN..]);
        Self { enc, mac }
    }

    /// The masterkey from its two unwrapped halves.
    pub(crate) fn from_parts(
        enc: Zeroizing<[u8; SUBKEY_LEN]>,
        mac: Zeroizing<[u8; SUBKEY_LEN]>,
    ) -> Self {
        Self { enc, mac }
    }

    pub(crate) fn enc_key(&self) -> &[u8; SUBKEY_LEN] {
        &self.enc
    }

    pub(crate) fn mac_key(&self) -> &[u8; SUBKEY_LEN] {
        &self.mac
    }

    /// Encryption key ‖ MAC key: the signing key of `vault.cryptomator`.
    pub(crate) fn raw(&self) -> Zeroizing<[u8; MASTERKEY_LEN]> {
        let mut raw = Zeroizing::new([0u8; MASTERKEY_LEN]);
        raw[..SUBKEY_LEN].copy_from_slice(&self.enc[..]);
        raw[SUBKEY_LEN..].copy_from_slice(&self.mac[..]);
        raw
    }

    /// MAC key ‖ encryption key: the AES-SIV key (RFC 5297 puts the S2V key
    /// first; Cryptomator's `siv-mode` combines `macKey ‖ ctrKey`).
    pub(crate) fn siv_key(&self) -> Zeroizing<[u8; MASTERKEY_LEN]> {
        let mut key = Zeroizing::new([0u8; MASTERKEY_LEN]);
        key[..SUBKEY_LEN].copy_from_slice(&self.mac[..]);
        key[SUBKEY_LEN..].copy_from_slice(&self.enc[..]);
        key
    }
}

impl fmt::Debug for Masterkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Masterkey(..)")
    }
}

/// Fills `buffer` from the system random generator.
pub(crate) fn fill_random(buffer: &mut [u8]) -> Result<(), FormatError> {
    getrandom::getrandom(buffer).map_err(|error| FormatError::Random(error.to_string()))
}

/// The masterkey of one unlocked vault, shared by everything that reads or
/// writes it (C3.2). `wipe` takes the key out and wipes it; every later use,
/// also through readers and writers that are already open, fails with
/// `FormatError::Locked` instead of using a key copy.
pub(crate) struct MasterkeyCell {
    key: RwLock<Option<Masterkey>>,
}

impl MasterkeyCell {
    pub(crate) fn new(key: Masterkey) -> Arc<Self> {
        Arc::new(Self {
            key: RwLock::new(Some(key)),
        })
    }

    /// Runs `use_key` with the masterkey; `Locked` once wiped.
    pub(crate) fn with<T>(&self, use_key: impl FnOnce(&Masterkey) -> T) -> Result<T, FormatError> {
        let guard = self.key.read().unwrap_or_else(PoisonError::into_inner);
        guard.as_ref().map(use_key).ok_or(FormatError::Locked)
    }

    /// Takes the masterkey out and wipes it (vault locked).
    pub(crate) fn wipe(&self) {
        let taken = self
            .key
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(taken);
    }

    pub(crate) fn is_wiped(&self) -> bool {
        self.key
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }
}

impl fmt::Debug for MasterkeyCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MasterkeyCell")
            .field("wiped", &self.is_wiped())
            .finish()
    }
}
