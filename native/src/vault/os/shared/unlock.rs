//! Unlocking (C3.1, C3.5): read `vault.cryptomator` and
//! `masterkey.cryptomator` from the vault root on its storage, check the key
//! source, derive the key-encryption key from the NFC password with the
//! scrypt limits and the available memory (C1.3), unwrap the masterkey,
//! verify the signature, then the format. The session goes into the session
//! register, so one unlock serves every surface of the device; the password
//! is remembered only after it unlocked the vault.
use std::fmt;
use std::io::{self, Read};
use std::sync::Arc;

use crate::cryptomator::{
    read_bounded, FormatError, Masterkey, MasterkeyFile, UnverifiedConfig, VaultConfig,
    MASTERKEY_FILE, METADATA_READ_LIMIT, VAULT_CONFIG_FILE,
};
use crate::vault::core::path_map::storage_path;
use crate::vault::core::VaultSession;
use crate::vfs::Backend;

use super::open::ResolveLocation;
use super::registry_store::RegistryStore;
use super::remember::{self, SecretStore};
use super::session::SessionRegister;

/// The only vault format se opens (C1.5).
const FORMAT: i64 = 8;

/// Why a site did not unlock (C3.1 texts). Texts carry no path and no name
/// from inside the vault (K24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UnlockError {
    /// No entered site of that name, or several.
    Unknown(String),
    /// The storage location could not be reached or read.
    Unreachable(String),
    /// Neither `vault.cryptomator` nor a key file of an older format.
    NotAVault,
    /// The key files say no: wrong password, Hub, format, memory, damage.
    Format(FormatError),
    /// The location holds another vault than the one entered.
    OtherVault,
    /// `--saved-password` without a remembered password.
    NoSavedPassword,
}

impl UnlockError {
    /// The structured code for IPC `Err.code` and the Bridge (C7.5).
    pub(crate) fn code(&self) -> Option<&'static str> {
        match self {
            Self::Format(FormatError::WrongPassword) => Some("vault_wrong_password"),
            Self::Format(FormatError::UnsupportedFormat(_)) => Some("vault_unsupported_format"),
            Self::Format(FormatError::HubVault) => Some("vault_hub"),
            Self::Format(FormatError::KdfMemory { .. }) => Some("vault_memory"),
            Self::NoSavedPassword => Some("vault_no_saved_password"),
            _ => None,
        }
    }
}

impl fmt::Display for UnlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(message) => f.write_str(message),
            Self::Unreachable(cause) => write!(f, "Speicherort nicht erreichbar: {cause}"),
            Self::NotAVault => f.write_str("Kein Cryptomator-Tresor (vault.cryptomator fehlt)"),
            Self::Format(error) => error.fmt(f),
            Self::OtherVault => {
                f.write_str("Am Speicherort liegt ein anderer Tresor als eingetragen")
            }
            Self::NoSavedPassword => f.write_str("Für diesen Tresor ist kein Passwort gemerkt"),
        }
    }
}

impl std::error::Error for UnlockError {}

/// The key files of a vault root as stored, read without a password.
pub(crate) struct KeyFiles {
    pub(crate) config: Option<UnverifiedConfig>,
    pub(crate) masterkey: Option<Vec<u8>>,
}

/// A metadata file of the vault root, at most 64 KiB; `None` when missing.
fn read_metadata(inner: &dyn Backend, path: &str) -> Result<Option<Vec<u8>>, UnlockError> {
    let unreachable = |error: io::Error| UnlockError::Unreachable(error.to_string());
    let reader: Box<dyn Read + Send> = match inner.open_read(path) {
        Ok(reader) => reader,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unreachable(error)),
    };
    match read_bounded(reader, METADATA_READ_LIMIT) {
        Ok(Some(bytes)) => Ok(Some(bytes)),
        // A larger file is no valid key file or configuration (C1.3).
        Ok(None) => Err(UnlockError::Format(FormatError::MasterkeyFileTooLarge)),
        Err(error) => Err(unreachable(error)),
    }
}

/// Reads both key files of the vault root `root`.
pub(crate) fn read_key_files(inner: &dyn Backend, root: &str) -> Result<KeyFiles, UnlockError> {
    let config = match read_metadata(inner, &storage_path(root, VAULT_CONFIG_FILE)) {
        Ok(Some(bytes)) => {
            Some(UnverifiedConfig::read_from(bytes.as_slice()).map_err(UnlockError::Format)?)
        }
        Ok(None) => None,
        Err(UnlockError::Format(FormatError::MasterkeyFileTooLarge)) => {
            return Err(UnlockError::Format(FormatError::ConfigTooLarge))
        }
        Err(error) => return Err(error),
    };
    let masterkey = read_metadata(inner, &storage_path(root, MASTERKEY_FILE))?;
    Ok(KeyFiles { config, masterkey })
}

/// The configuration of a format-8 vault; without one, the reason: a key
/// file of format 7 or older asks for one opening in Cryptomator (C1.5).
pub(crate) fn require_config(files: &KeyFiles) -> Result<&UnverifiedConfig, UnlockError> {
    if let Some(config) = &files.config {
        return Ok(config);
    }
    match files.masterkey.as_deref().map(MasterkeyFile::parse) {
        Some(Ok(file)) if file.version() < FORMAT => Err(UnlockError::Format(
            FormatError::UnsupportedFormat(file.version()),
        )),
        _ => Err(UnlockError::NotAVault),
    }
}

/// The parsed key file of a format-8 vault.
pub(crate) fn require_key_file(files: &KeyFiles) -> Result<MasterkeyFile, UnlockError> {
    let bytes = files.masterkey.as_deref().ok_or_else(|| {
        UnlockError::Format(FormatError::InvalidMasterkeyFile(
            "masterkey.cryptomator fehlt".into(),
        ))
    })?;
    MasterkeyFile::parse(bytes).map_err(UnlockError::Format)
}

/// Unwraps the masterkey of the vault at `root` and verifies its
/// configuration, in the order of C3.1: JWT, key source, key file, key,
/// signature, format. `available` is the memory scrypt may take.
pub(crate) fn unlock_storage(
    inner: &dyn Backend,
    root: &str,
    password: &str,
    available: Option<u64>,
) -> Result<(Masterkey, VaultConfig), UnlockError> {
    let files = read_key_files(inner, root)?;
    let config = require_config(&files)?;
    // Hub and foreign key sources are refused before any key is loaded.
    config.key_source().map_err(UnlockError::Format)?;
    let key_file = require_key_file(&files)?;
    let key = key_file
        .unlock(password, available)
        .map_err(UnlockError::Format)?;
    let verified = config.verify(&key).map_err(UnlockError::Format)?;
    Ok((key, verified))
}

/// Options of one unlock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UnlockOptions {
    /// `Some(true)` remembers the password, `Some(false)` forgets it.
    pub(crate) remember: Option<bool>,
    /// Memory scrypt may take (desktop: available memory of the system;
    /// Android: `memAvail − memThreshold` from the app, C1.3 (c)).
    pub(crate) available: Option<u64>,
}

/// The result of an unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Unlocked {
    pub(crate) site: String,
    pub(crate) epoch: u64,
    /// The site was unlocked already; nothing was derived again.
    pub(crate) already: bool,
    /// The unlock stands, but remembering or forgetting failed.
    pub(crate) remember_error: Option<String>,
}

/// Constant-time comparison of two raw masterkeys.
fn same_key(left: &Masterkey, right: &Masterkey) -> bool {
    let (left, right) = (left.raw(), right.raw());
    left.iter()
        .zip(right.iter())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// Unlocks the entered site `query` (ID, ID prefix or label) in `register`.
/// An unlocked site stays as it is; the password is then checked only when
/// it is to be remembered, so a wrong one is never stored.
pub(crate) fn unlock_entered(
    register: &SessionRegister,
    store: &RegistryStore,
    secrets: &dyn SecretStore,
    query: &str,
    password: &str,
    options: UnlockOptions,
    resolve: ResolveLocation<'_>,
) -> Result<Unlocked, UnlockError> {
    let record = store.resolve(query).map_err(UnlockError::Unknown)?;
    if let Some((session, epoch)) = register.session(&record.id) {
        if options.remember == Some(true) {
            let (inner, root) = resolve(&record.location).map_err(UnlockError::Unreachable)?;
            let (key, _) = unlock_storage(inner.as_ref(), &root, password, options.available)?;
            let same = session.with_key(|current| Ok(same_key(current, &key)));
            if !matches!(same, Ok(true)) {
                return Err(UnlockError::Format(FormatError::WrongPassword));
            }
        }
        let remember_error =
            remember::apply_choice(secrets, store, &record, password, options.remember);
        return Ok(Unlocked {
            site: record.id,
            epoch,
            already: true,
            remember_error,
        });
    }
    let (inner, root) = resolve(&record.location).map_err(UnlockError::Unreachable)?;
    let (key, config) = unlock_storage(inner.as_ref(), &root, password, options.available)?;
    if !record.vault_id.is_empty() && config.jti != record.vault_id {
        return Err(UnlockError::OtherVault);
    }
    let installed = register.install(&record, VaultSession::new(&record.id, key, config));
    let remember_error = if installed.already {
        None
    } else {
        remember::apply_choice(secrets, store, &record, password, options.remember)
    };
    Ok(Unlocked {
        site: record.id,
        epoch: installed.epoch,
        already: installed.already,
        remember_error,
    })
}

/// Unlocks with the remembered password (`UnlockSaved`, `--saved-password`).
pub(crate) fn unlock_saved(
    register: &SessionRegister,
    store: &RegistryStore,
    secrets: &dyn SecretStore,
    query: &str,
    available: Option<u64>,
    resolve: ResolveLocation<'_>,
) -> Result<Unlocked, UnlockError> {
    let record = store.resolve(query).map_err(UnlockError::Unknown)?;
    if let Some((_, epoch)) = register.session(&record.id) {
        return Ok(Unlocked {
            site: record.id,
            epoch,
            already: true,
            remember_error: None,
        });
    }
    let password = remember::saved_password(secrets, &record.id)
        .map_err(UnlockError::Unreachable)?
        .ok_or(UnlockError::NoSavedPassword)?;
    let options = UnlockOptions {
        remember: None,
        available,
    };
    unlock_entered(
        register, store, secrets, &record.id, &password, options, resolve,
    )
}

/// The session of a freshly unlocked vault outside the register (tests and
/// the interop stage).
pub(crate) fn session_for(
    site: &str,
    inner: &dyn Backend,
    root: &str,
    password: &str,
) -> Result<Arc<VaultSession>, UnlockError> {
    let (key, config) = unlock_storage(inner, root, password, None)?;
    Ok(VaultSession::new(site, key, config))
}
