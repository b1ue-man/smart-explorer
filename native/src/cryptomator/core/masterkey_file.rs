//! `masterkey.cryptomator` (CryptoLib `MasterkeyFile`): JSON with the scrypt
//! salt and parameters, the two AES-KW-wrapped masterkey halves and the
//! legacy `version` with its `versionMac`. Reading is bounded to 64 KiB
//! (C1.3: an own limit of se; every valid file is smaller than 1 KiB).
//! `version` accepts every integer (C1.2); `versionMac` is only required to
//! be present, as in CryptoLib (format 8 protects downgrades by the
//! signature of `vault.cryptomator`).
use std::io::Read;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::content_chunks::hmac_sha256;
use super::kdf::{derive_kek, unwrap_key, wrap_key, WRAPPED_LEN};
use super::keys::{fill_random, Masterkey};
use super::vault_files::{read_bounded, METADATA_READ_LIMIT};
use super::FormatError;

/// scrypt cost parameter N of new key files (CryptoLib default).
pub(crate) const DEFAULT_COST: i64 = 1 << 15;
/// scrypt block size r of new key files (CryptoLib default).
pub(crate) const DEFAULT_BLOCK_SIZE: i64 = 8;
/// Salt length of new key files (CryptoLib default).
pub(crate) const DEFAULT_SALT_LEN: usize = 8;
/// The legacy `version` written since format 8 (CryptoLib).
pub(crate) const LEGACY_VERSION: i64 = 999;

#[derive(Deserialize)]
struct Stored {
    version: Option<i64>,
    #[serde(rename = "scryptSalt")]
    salt: Option<String>,
    #[serde(rename = "scryptCostParam")]
    cost: Option<i64>,
    #[serde(rename = "scryptBlockSize")]
    block_size: Option<i64>,
    #[serde(rename = "primaryMasterKey")]
    primary: Option<String>,
    #[serde(rename = "hmacMasterKey")]
    hmac: Option<String>,
    #[serde(rename = "versionMac")]
    version_mac: Option<String>,
}

#[derive(Serialize)]
struct Written<'a> {
    version: i64,
    #[serde(rename = "scryptSalt")]
    salt: &'a str,
    #[serde(rename = "scryptCostParam")]
    cost: i64,
    #[serde(rename = "scryptBlockSize")]
    block_size: i64,
    #[serde(rename = "primaryMasterKey")]
    primary: &'a str,
    #[serde(rename = "hmacMasterKey")]
    hmac: &'a str,
    #[serde(rename = "versionMac")]
    version_mac: &'a str,
}

/// A parsed and structurally valid key file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MasterkeyFile {
    version: i64,
    salt: Vec<u8>,
    cost: i64,
    block_size: i64,
    wrapped_enc: [u8; WRAPPED_LEN],
    wrapped_mac: [u8; WRAPPED_LEN],
    version_mac: Vec<u8>,
}

fn invalid(detail: &str) -> FormatError {
    FormatError::InvalidMasterkeyFile(detail.to_string())
}

fn decode(field: Option<String>, name: &str) -> Result<Vec<u8>, FormatError> {
    let text = field.ok_or_else(|| invalid(&format!("{name} fehlt")))?;
    STANDARD
        .decode(text.as_bytes())
        .map_err(|_| invalid(&format!("{name} ist kein Base64")))
}

fn wrapped(field: Option<String>, name: &str) -> Result<[u8; WRAPPED_LEN], FormatError> {
    decode(field, name)?
        .as_slice()
        .try_into()
        .map_err(|_| invalid(&format!("{name} hat nicht {WRAPPED_LEN} Byte")))
}

impl MasterkeyFile {
    /// Parses the bytes of a key file (at most `METADATA_READ_LIMIT`).
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() > METADATA_READ_LIMIT {
            return Err(FormatError::MasterkeyFileTooLarge);
        }
        let stored: Stored =
            serde_json::from_slice(bytes).map_err(|_| invalid("kein JSON-Objekt"))?;
        let version = stored.version.ok_or_else(|| invalid("version fehlt"))?;
        let cost = stored
            .cost
            .ok_or_else(|| invalid("scryptCostParam fehlt"))?;
        let block_size = stored
            .block_size
            .ok_or_else(|| invalid("scryptBlockSize fehlt"))?;
        if cost <= 1 || block_size <= 0 {
            return Err(invalid("scrypt-Parameter ungültig"));
        }
        Ok(Self {
            version,
            salt: decode(stored.salt, "scryptSalt")?,
            cost,
            block_size,
            wrapped_enc: wrapped(stored.primary, "primaryMasterKey")?,
            wrapped_mac: wrapped(stored.hmac, "hmacMasterKey")?,
            version_mac: decode(stored.version_mac, "versionMac")?,
        })
    }

    /// Reads and parses a key file; reads at most 64 KiB plus one byte.
    pub(crate) fn read_from(reader: impl Read) -> Result<Self, FormatError> {
        match read_bounded(reader, METADATA_READ_LIMIT) {
            Ok(Some(bytes)) => Self::parse(&bytes),
            Ok(None) => Err(FormatError::MasterkeyFileTooLarge),
            Err(error) => Err(invalid(&format!("nicht lesbar ({})", error.kind()))),
        }
    }

    /// The legacy `version` field (≤ 7 marks a vault before format 8).
    pub(crate) fn version(&self) -> i64 {
        self.version
    }

    /// scrypt cost parameter N and block size r.
    pub(crate) fn kdf_params(&self) -> (i64, i64) {
        (self.cost, self.block_size)
    }

    /// Unwraps the masterkey with `password`. `available` is the memory
    /// this process may give scrypt (`None` = only the fixed limits).
    pub(crate) fn unlock(
        &self,
        password: &str,
        available: Option<u64>,
    ) -> Result<Masterkey, FormatError> {
        let kek = derive_kek(password, &self.salt, self.cost, self.block_size, available)?;
        let enc = unwrap_key(&kek, &self.wrapped_enc)?;
        let mac = unwrap_key(&kek, &self.wrapped_mac)?;
        Ok(Masterkey::from_parts(enc, mac))
    }

    /// The bytes of a new key file for `key` and `password`: random 8-byte
    /// salt, N = 2^15, r = 8, `version` 999 (CryptoLib defaults).
    pub(crate) fn create(
        key: &Masterkey,
        password: &str,
        available: Option<u64>,
    ) -> Result<Vec<u8>, FormatError> {
        let mut salt = [0u8; DEFAULT_SALT_LEN];
        fill_random(&mut salt)?;
        Self::create_with(
            key,
            password,
            &salt,
            DEFAULT_COST,
            DEFAULT_BLOCK_SIZE,
            LEGACY_VERSION,
            available,
        )
    }

    /// The bytes of a key file with chosen salt, parameters and version
    /// (`create`, password change, CryptoLib vectors).
    pub(super) fn create_with(
        key: &Masterkey,
        password: &str,
        salt: &[u8],
        cost: i64,
        block_size: i64,
        version: i64,
        available: Option<u64>,
    ) -> Result<Vec<u8>, FormatError> {
        let legacy = i32::try_from(version).map_err(|_| invalid("version außerhalb von int32"))?;
        let version_mac = hmac_sha256(key.mac_key(), &[&legacy.to_be_bytes()[..]])?;
        Self::write(
            key,
            password,
            salt,
            cost,
            block_size,
            version,
            &version_mac,
            available,
        )
    }

    /// Password change (CryptoLib `changePassphrase`): unwrap with
    /// `old_password`, wrap again with `new_password` under a new random
    /// salt; N, r, `version` and `versionMac` stay, the masterkey stays.
    pub(crate) fn rewrap(
        &self,
        old_password: &str,
        new_password: &str,
        available: Option<u64>,
    ) -> Result<Vec<u8>, FormatError> {
        let key = self.unlock(old_password, available)?;
        let mut salt = [0u8; DEFAULT_SALT_LEN];
        fill_random(&mut salt)?;
        Self::write(
            &key,
            new_password,
            &salt,
            self.cost,
            self.block_size,
            self.version,
            &self.version_mac,
            available,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        key: &Masterkey,
        password: &str,
        salt: &[u8],
        cost: i64,
        block_size: i64,
        version: i64,
        version_mac: &[u8],
        available: Option<u64>,
    ) -> Result<Vec<u8>, FormatError> {
        let kek = derive_kek(password, salt, cost, block_size, available)?;
        let primary = wrap_key(&kek, key.enc_key())?;
        let hmac = wrap_key(&kek, key.mac_key())?;
        drop(kek);
        let salt = STANDARD.encode(salt);
        let primary = STANDARD.encode(primary);
        let hmac = STANDARD.encode(hmac);
        let version_mac = STANDARD.encode(version_mac);
        let written = Written {
            version,
            salt: &salt,
            cost,
            block_size,
            primary: &primary,
            hmac: &hmac,
            version_mac: &version_mac,
        };
        // Only wrapped keys and public parameters: nothing here is secret.
        serde_json::to_vec_pretty(&written).map_err(|_| invalid("nicht schreibbar"))
    }
}
