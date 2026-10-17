//! Format 8 building blocks and the one error type they share. No I/O of its
//! own beyond `std::io::Read`/`Write` adapters, no platform code.
use std::fmt;
use std::io;

mod cipher_combo;
mod content_chunks;
mod content_header;
mod content_stream;
mod dir_ids;
mod kdf;
mod keys;
mod masterkey_file;
mod names;
mod vault_config;
mod vault_files;

pub(crate) use cipher_combo::{
    ciphertext_size, cleartext_file_size, cleartext_size, file_size, CipherCombo, CHUNK_PAYLOAD,
};
pub(crate) use content_header::FileHeader;
pub(crate) use content_stream::{
    decrypt_bytes, encrypt_bytes, ContentCryptor, DecryptingReader, EncryptingWriter,
};
pub(crate) use dir_ids::{
    content_dir, dir_file_bytes, dir_id_hash, new_dir_id, parse_dir_file, ROOT_DIR_ID,
};
pub(crate) use kdf::required_memory;
pub(crate) use keys::{Masterkey, MasterkeyCell};
pub(crate) use masterkey_file::MasterkeyFile;
pub(crate) use names::{decrypt_name, encrypt_name, needs_shortening, shortened_name};
pub(crate) use vault_config::{KeySource, UnverifiedConfig, VaultConfig};
pub(crate) use vault_files::{
    backup_file_name, backup_suffix, is_metadata_name, read_bounded, BACKUP_SUFFIX, CIPHER_SUFFIX,
    CONTENTS_FILE, DATA_DIR, DEFAULT_SHORTENING_THRESHOLD, DIR_FILE, DIR_FILE_READ_LIMIT,
    DIR_ID_BACKUP_FILE, INFLATED_NAME_FILE, INFLATED_NAME_READ_LIMIT, INUSE_SUFFIX, MASTERKEY_FILE,
    MASTERKEY_FILE_KEY_ID, MAX_SHORTENING_THRESHOLD, MAX_SYMLINK_TARGET_CHARS, METADATA_READ_LIMIT,
    MIN_SHORTENING_THRESHOLD, SHORTENED_SUFFIX, SYMLINK_FILE, VAULT_CONFIG_FILE,
};

const MIB: u64 = 1024 * 1024;

/// Why vault bytes could not be read or written. Texts name no file name and
/// no path (crash log rule K24); the vault layer adds the site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FormatError {
    /// The key file did not unwrap: wrong password (or a damaged key file).
    WrongPassword,
    /// The key file is not a Cryptomator key file.
    InvalidMasterkeyFile(String),
    /// The key file is larger than `METADATA_READ_LIMIT`.
    MasterkeyFileTooLarge,
    /// `vault.cryptomator` is larger than `METADATA_READ_LIMIT`.
    ConfigTooLarge,
    /// The scrypt parameters exceed the limits of C1.3 (a) or (b).
    KdfLimits(String),
    /// The scrypt memory exceeds what this process may allocate (C1.3 (c)).
    KdfMemory { needed: u64, available: u64 },
    /// `format` of the vault configuration (or a legacy key file) is not 8.
    UnsupportedFormat(i64),
    /// The key comes from Cryptomator Hub (`kid` = `hub+…`).
    HubVault,
    /// The key comes from anywhere else than the fixed key file.
    UnsupportedKeySource,
    /// `vault.cryptomator` is not a usable vault configuration.
    InvalidConfig(String),
    /// The signature of `vault.cryptomator` does not match the masterkey.
    ConfigSignature,
    /// An encrypted name is malformed.
    InvalidName,
    /// A directory ID (or its file `dir.c9r`) is malformed.
    InvalidDirId,
    /// A file header has the wrong size.
    InvalidHeader,
    /// A content chunk is shorter than its nonce and tag or longer than allowed.
    InvalidChunk,
    /// A ciphertext size that no cleartext size produces.
    InvalidSize,
    /// A tag or MAC did not verify; nothing of this item was decrypted.
    Authentication(&'static str),
    /// A cleartext item is larger than the format allows.
    TooLarge(&'static str),
    /// The vault was locked: its masterkey is wiped.
    Locked,
    /// The system random generator failed.
    Random(String),
    /// A primitive refused its input (lengths are checked before, so this
    /// indicates a broken invariant, never a decryption result).
    Crypto(&'static str),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPassword => f.write_str("Passwort falsch"),
            Self::InvalidMasterkeyFile(detail) => write!(f, "Schlüsseldatei ungültig: {detail}"),
            Self::MasterkeyFileTooLarge => {
                f.write_str("Schlüsseldatei größer als 64 KiB – kein gültiger Tresor")
            }
            Self::ConfigTooLarge => {
                f.write_str("Tresorkonfiguration größer als 64 KiB – kein gültiger Tresor")
            }
            Self::KdfLimits(detail) => write!(f, "Schlüsselableitung nicht zulässig: {detail}"),
            Self::KdfMemory { needed, available } => write!(
                f,
                "Schlüsselableitung braucht {} MiB, verfügbar {} MiB",
                needed.div_ceil(MIB),
                available / MIB
            ),
            Self::UnsupportedFormat(format) if *format < 8 => write!(
                f,
                "Tresorformat {format} nicht unterstützt: den Tresor einmal in Cryptomator \
                 öffnen, Cryptomator aktualisiert ihn auf Format 8"
            ),
            Self::UnsupportedFormat(format) => write!(f, "Tresorformat {format} nicht unterstützt"),
            Self::HubVault => f.write_str("Cryptomator-Hub-Tresore werden nicht unterstützt"),
            Self::UnsupportedKeySource => f.write_str("Nicht unterstützte Schlüsselquelle"),
            Self::InvalidConfig(detail) => write!(f, "Tresorkonfiguration ungültig: {detail}"),
            Self::ConfigSignature => f.write_str("Signatur der Tresorkonfiguration ungültig"),
            Self::InvalidName => f.write_str("Verschlüsselter Name ungültig"),
            Self::InvalidDirId => f.write_str("Verzeichnis-ID ungültig"),
            Self::InvalidHeader => f.write_str("Dateikopf ungültig"),
            Self::InvalidChunk => f.write_str("Verschlüsselter Block ungültig"),
            Self::InvalidSize => f.write_str("Chiffratgröße ungültig"),
            Self::Authentication(what) => {
                write!(
                    f,
                    "{what} nicht authentisch (beschädigt oder falscher Schlüssel)"
                )
            }
            Self::TooLarge(what) => write!(f, "{what} größer als erlaubt"),
            Self::Locked => f.write_str("Tresor gesperrt"),
            Self::Random(detail) => write!(f, "Zufallszahlen nicht verfügbar: {detail}"),
            Self::Crypto(what) => write!(f, "Kryptobaustein {what} fehlgeschlagen"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<FormatError> for io::Error {
    /// The error stays the source (`get_ref`), so callers can tell a locked
    /// vault (`PermissionDenied`, C3.2) from damaged bytes (`InvalidData`).
    fn from(error: FormatError) -> Self {
        let kind = match &error {
            FormatError::WrongPassword | FormatError::Locked => io::ErrorKind::PermissionDenied,
            FormatError::KdfMemory { .. } => io::ErrorKind::OutOfMemory,
            FormatError::HubVault
            | FormatError::UnsupportedKeySource
            | FormatError::UnsupportedFormat(_) => io::ErrorKind::Unsupported,
            FormatError::Random(_) | FormatError::Crypto(_) => io::ErrorKind::Other,
            _ => io::ErrorKind::InvalidData,
        };
        io::Error::new(kind, error)
    }
}

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod content_tests;
#[cfg(test)]
mod names_tests;
#[cfg(test)]
mod vectors_tests;
