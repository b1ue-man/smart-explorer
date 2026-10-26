//! Local sealing with purpose keys (C3.6). From the masterkey of an
//! unlocked vault (encryption key ‖ MAC key) HKDF-SHA256 derives one key per
//! purpose:
//!
//! - `se-vault-local-store-v1`: data of se about places inside the vault
//!   (`<AD>/vaults/<site-id>.local`, K13/K14),
//! - `se-vault-log-fields-v1`: path fields in protocols (K20/K21),
//! - `se-vault-transfer-job-v1` and `se-vault-transfer-alias-v1`: durable
//!   transfer records containing names, bound to their exact participating roots,
//! - `se-vault-spool-wrap-v1`: the key that wraps the spool key of a vault
//!   drive (AES-KW, V1); it seals nothing itself and never leaves the worker.
//!
//! Sealing always uses the vault content format (header plus AES-256-GCM
//! chunks, `SIV_GCM`); short texts travel as Base64url of it. A purpose key
//! lives only inside one call in `Zeroizing` buffers, so a lock leaves none
//! behind.
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::cryptomator::{
    decrypt_bytes, encrypt_bytes, CipherCombo, ContentCryptor, FormatError, Masterkey,
    MasterkeyCell,
};

/// Largest cleartext of one sealed text item (a path field).
pub(crate) const MAX_SEALED_TEXT: usize = 64 * 1024;

/// What a purpose key is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SealPurpose {
    LocalStore,
    LogFields,
    SpoolWrap,
    TransferJob,
    TransferAlias,
}

impl SealPurpose {
    pub(crate) const ALL: [Self; 5] = [
        Self::LocalStore,
        Self::LogFields,
        Self::SpoolWrap,
        Self::TransferJob,
        Self::TransferAlias,
    ];

    /// The HKDF `info` of the purpose (C3.6).
    pub(crate) fn info(self) -> &'static str {
        match self {
            Self::LocalStore => "se-vault-local-store-v1",
            Self::LogFields => "se-vault-log-fields-v1",
            Self::SpoolWrap => "se-vault-spool-wrap-v1",
            Self::TransferJob => "se-vault-transfer-job-v1",
            Self::TransferAlias => "se-vault-transfer-alias-v1",
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LocalStore => "local_store",
            Self::LogFields => "log_fields",
            Self::SpoolWrap => "spool_wrap",
            Self::TransferJob => "transfer_job",
            Self::TransferAlias => "transfer_alias",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|purpose| purpose.as_str() == text)
    }

    /// Whether processes without the key (GUI, CLI) may seal or unseal with
    /// it over IPC (`Seal`, `Unseal`); the spool wrap key stays in the worker.
    pub(crate) fn over_ipc(self) -> bool {
        !matches!(self, Self::SpoolWrap)
    }
}

/// HKDF-SHA256 over encryption key ‖ MAC key without salt, `info` of the
/// purpose, `out.len()` bytes.
fn derive(masterkey: &Masterkey, purpose: SealPurpose, out: &mut [u8]) -> Result<(), FormatError> {
    let input = masterkey.raw();
    Hkdf::<Sha256>::new(None, &input[..])
        .expand(purpose.info().as_bytes(), out)
        .map_err(|_| FormatError::Crypto("HKDF"))
}

/// The 256-bit key that wraps the spool keys of the vault's drives (AES-KW,
/// C3.6). Used by the worker only, inside one call.
pub(crate) fn spool_wrap_key(masterkey: &Masterkey) -> Result<Zeroizing<[u8; 32]>, FormatError> {
    let mut key = Zeroizing::new([0u8; 32]);
    derive(masterkey, SealPurpose::SpoolWrap, &mut key[..])?;
    Ok(key)
}

/// Seals and unseals in the vault content format under one purpose key.
/// Holds its own copy of the purpose key; dropped at the end of the call
/// that made it, which wipes the key.
pub(crate) struct Sealer {
    cryptor: ContentCryptor,
}

impl Sealer {
    /// The sealer of `purpose`; the spool wrap key is no sealing purpose.
    pub(crate) fn new(masterkey: &Masterkey, purpose: SealPurpose) -> Result<Self, FormatError> {
        if purpose == SealPurpose::SpoolWrap {
            return Err(FormatError::Crypto("spool-wrap versiegelt keine Inhalte"));
        }
        // Encryption key ‖ MAC key of the content format; `SIV_GCM` reads
        // only the first half (header key), the second is derived for shape.
        let mut raw = Zeroizing::new([0u8; 64]);
        derive(masterkey, purpose, &mut raw[..])?;
        let key = Masterkey::from_raw(&raw);
        Ok(Self {
            cryptor: ContentCryptor::new(MasterkeyCell::new(key), CipherCombo::SivGcm),
        })
    }

    pub(crate) fn seal(&self, cleartext: &[u8]) -> Result<Vec<u8>, FormatError> {
        encrypt_bytes(&self.cryptor, cleartext)
    }

    /// The cleartext of `sealed`, refused above `limit` bytes; a tag that
    /// does not verify (another purpose, another vault, changed bytes) yields
    /// no byte.
    pub(crate) fn unseal(
        &self,
        sealed: &[u8],
        limit: usize,
    ) -> Result<Zeroizing<Vec<u8>>, FormatError> {
        decrypt_bytes(&self.cryptor, sealed, limit)
    }

    /// `text` sealed as Base64url without padding.
    pub(crate) fn seal_text(&self, text: &str) -> Result<String, FormatError> {
        if text.len() > MAX_SEALED_TEXT {
            return Err(FormatError::TooLarge("Zu versiegelnder Text"));
        }
        Ok(URL_SAFE_NO_PAD.encode(self.seal(text.as_bytes())?))
    }

    /// The text of a `seal_text` result.
    pub(crate) fn unseal_text(&self, sealed: &str) -> Result<Zeroizing<String>, FormatError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(sealed.trim().trim_end_matches('=').as_bytes())
            .map_err(|_| FormatError::InvalidHeader)?;
        let plain = self.unseal(&bytes, MAX_SEALED_TEXT)?;
        // Only `seal` of non-text bytes gets here; never a failed tag.
        let text =
            std::str::from_utf8(&plain).map_err(|_| FormatError::Crypto("Versiegelter Text"))?;
        Ok(Zeroizing::new(text.to_owned()))
    }
}
