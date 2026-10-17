//! Changing the password of a vault (VLT-M5, C3.7, C7.1 T10): only the
//! key file changes; the masterkey, the configuration and every file stay.
//!
//! Never without a valid masterkey: the new key file is computed in memory
//! (`MasterkeyFile::rewrap`, new salt, same scrypt cost), unlocked there with
//! the new password, and its masterkey must verify the signature of
//! `vault.cryptomator`; only then is anything written. Never a moment
//! without a valid key file: the old bytes are kept as
//! `masterkey.cryptomator.<8 HEX>.bkup` (as Cryptomator names it), the new
//! bytes go into a stage of the storage and replace `masterkey.cryptomator`
//! in one atomic step (`promote_staged`). A storage without an atomic
//! replace keeps the old key file and refuses the change.
use std::fmt;
use std::io::{self, Write};

use icu_normalizer::ComposingNormalizerBorrowed;

use crate::cryptomator::{backup_file_name, FormatError, MasterkeyFile, MASTERKEY_FILE};
use crate::vault::core::path_map::storage_path;
use crate::vfs::{self, Backend};

use super::open::ResolveLocation;
use super::registry_store::RegistryStore;
use super::remember::{self, SecretStore};
use super::unlock::{read_key_files, require_config, require_key_file, UnlockError};

/// Shortest new password in characters (NFC), as Cryptomator
/// `DEFAULT_MIN_PW_LENGTH` (C7.6, C7.7).
pub(crate) const MIN_PASSWORD_CHARS: usize = 8;

/// Checks a new password (creating a vault, changing the password).
pub(crate) fn check_new_password(password: &str) -> Result<(), String> {
    let normalized = ComposingNormalizerBorrowed::new_nfc().normalize(password);
    if normalized.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!(
            "Das Passwort braucht mindestens {MIN_PASSWORD_CHARS} Zeichen"
        ));
    }
    Ok(())
}

/// A changed password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PasswordChanged {
    /// The name of the backup of the old key file in the vault root.
    pub(crate) backup: String,
    /// The password changed, but updating the remembered one failed.
    pub(crate) remember_error: Option<String>,
}

/// Why the password stayed as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PasswordError {
    /// The new password is too short.
    NewPassword(String),
    /// The vault did not open with the old password (or is no vault).
    Unlock(UnlockError),
    /// The storage cannot replace the key file atomically.
    NotAtomic,
    /// Reading or writing the storage failed.
    Storage(String),
}

impl PasswordError {
    pub(crate) fn code(&self) -> Option<&'static str> {
        match self {
            Self::Unlock(error) => error.code(),
            Self::NotAtomic => Some("vault_not_atomic"),
            _ => None,
        }
    }
}

impl fmt::Display for PasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewPassword(message) | Self::Storage(message) => f.write_str(message),
            Self::Unlock(error) => error.fmt(f),
            Self::NotAtomic => f.write_str(
                "Der Speicherort kann masterkey.cryptomator nicht atomar ersetzen; \
                 das Passwort bleibt unverändert",
            ),
        }
    }
}

impl std::error::Error for PasswordError {}

fn storage(error: io::Error) -> PasswordError {
    PasswordError::Storage(format!("Speicherort: {error}"))
}

fn key_error(error: FormatError) -> PasswordError {
    PasswordError::Unlock(UnlockError::Format(error))
}

/// Writes a new file exclusively (`AlreadyExists` when the name is taken);
/// a storage without exclusive creation writes after an absence check, as
/// the vault backend writes its structure files.
pub(crate) fn write_new(inner: &dyn Backend, path: &str, bytes: &[u8]) -> io::Result<()> {
    let mut writer = match inner.open_write_copy_stage_sized(path, bytes.len() as u64) {
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            if inner.try_exists(path)? {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Datei existiert bereits",
                ));
            }
            inner.open_write(path)?
        }
        other => other?,
    };
    writer.write_all(bytes)?;
    writer.flush()
}

/// Writes the backup of the old key file; `Ok(true)` when this call made it.
fn write_backup(inner: &dyn Backend, path: &str, bytes: &[u8]) -> Result<bool, PasswordError> {
    match write_new(inner, path, bytes) {
        Ok(()) => Ok(true),
        // Same bytes, same name: the backup of an earlier change is there.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(storage(error)),
    }
}

fn discard(inner: &dyn Backend, stage: &str) {
    if inner.discard_copy_stage(stage).is_err() {
        let _ = inner.remove_file(stage);
    }
}

/// Puts `bytes` into place as the key file in one atomic replace.
fn replace_key_file(inner: &dyn Backend, target: &str, bytes: &[u8]) -> Result<(), PasswordError> {
    let stage = vfs::unique_staging_path(inner, target, "vault-key").map_err(storage)?;
    let written = write_new(inner, &stage, bytes);
    if let Err(error) = written {
        discard(inner, &stage);
        return Err(storage(error));
    }
    match inner.promote_staged(&stage, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            discard(inner, &stage);
            Err(if error.kind() == io::ErrorKind::Unsupported {
                PasswordError::NotAtomic
            } else {
                storage(error)
            })
        }
    }
}

/// Changes the password of the vault at `root` from `old` to `new`.
/// `expected_vault_id` (the `jti` of the entry) guards against another
/// vault at the same place; `available` is the memory scrypt may take.
pub(crate) fn change_password(
    inner: &dyn Backend,
    root: &str,
    expected_vault_id: Option<&str>,
    old: &str,
    new: &str,
    available: Option<u64>,
) -> Result<PasswordChanged, PasswordError> {
    check_new_password(new).map_err(PasswordError::NewPassword)?;
    let files = read_key_files(inner, root).map_err(PasswordError::Unlock)?;
    let config = require_config(&files).map_err(PasswordError::Unlock)?;
    config.key_source().map_err(key_error)?;
    let old_file = require_key_file(&files).map_err(PasswordError::Unlock)?;
    let old_bytes = files.masterkey.as_deref().unwrap_or_default();
    // A wrong old password ends here (unwrap fails), before any write.
    let new_bytes = old_file.rewrap(old, new, available).map_err(key_error)?;
    // The new key file must yield the masterkey that signs this vault.
    let key = MasterkeyFile::parse(&new_bytes)
        .and_then(|file| file.unlock(new, available))
        .map_err(key_error)?;
    let verified = config.verify(&key).map_err(key_error)?;
    drop(key);
    if expected_vault_id.is_some_and(|expected| !expected.is_empty() && expected != verified.jti) {
        return Err(PasswordError::Unlock(UnlockError::OtherVault));
    }
    let backup = backup_file_name(MASTERKEY_FILE, old_bytes);
    let backup_path = storage_path(root, &backup);
    let created_backup = write_backup(inner, &backup_path, old_bytes)?;
    if let Err(error) = replace_key_file(inner, &storage_path(root, MASTERKEY_FILE), &new_bytes) {
        if created_backup {
            // Nothing changed: no backup of a change that did not happen.
            let _ = inner.remove_file(&backup_path);
        }
        return Err(error);
    }
    Ok(PasswordChanged {
        backup,
        remember_error: None,
    })
}

/// `se vault password`, the desktop dialog, `vault.changePassword`: the
/// entered site `query`; a remembered password is replaced by the new one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn change_site_password(
    store: &RegistryStore,
    secrets: &dyn SecretStore,
    query: &str,
    old: &str,
    new: &str,
    available: Option<u64>,
    resolve: ResolveLocation<'_>,
) -> Result<PasswordChanged, PasswordError> {
    let record = store
        .resolve(query)
        .map_err(|message| PasswordError::Unlock(UnlockError::Unknown(message)))?;
    let (inner, root) = resolve(&record.location)
        .map_err(|cause| PasswordError::Unlock(UnlockError::Unreachable(cause)))?;
    let mut changed = change_password(
        inner.as_ref(),
        &root,
        Some(&record.vault_id),
        old,
        new,
        available,
    )?;
    if record.remember_password {
        changed.remember_error = remember::remember(secrets, store, &record, new).err();
    }
    Ok(changed)
}
