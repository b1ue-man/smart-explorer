//! „Passwort merken“ (C3.5, C7.1 T11): on request per site, off by default.
//! The password lives only in the secret store of the installation
//! (`crate::creds`: Windows Credential Manager; Linux and Android the sealed
//! file store, Android with its Keystore key) under the account
//! `smart-explorer:vault:<site-id>`. The vault list keeps only the flag
//! `remember_password`, which follows a stored secret: it is set after the
//! secret was stored and cleared after it was deleted. A password is stored
//! only after it unlocked the vault, and it is read only for unlocking
//! (CLI only with `--saved-password`).
use zeroize::Zeroizing;

use crate::vault::core::registry_types::SiteRecord;

use super::registry_store::RegistryStore;

/// Prefix of the secret store account of a site.
pub(crate) const ACCOUNT_PREFIX: &str = "smart-explorer:vault:";

/// The secret store account of `site` (a site ID).
pub(crate) fn account(site: &str) -> String {
    format!("{ACCOUNT_PREFIX}{site}")
}

/// The secret store as the vault layer uses it; tests replace it.
pub(crate) trait SecretStore: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    /// Deleting an account that holds nothing succeeds.
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The secret store of the installation (`crate::creds`).
pub(crate) struct OsSecretStore;

impl SecretStore for OsSecretStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        crate::creds::set_secret(account, secret)
    }

    fn get(&self, account: &str) -> Result<Option<String>, String> {
        crate::creds::get_secret_checked(account)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        crate::creds::delete_secret_checked(account)
    }
}

/// Saves the site's flag as `remember`, reading the current entry first so
/// a concurrent change of other settings survives.
fn set_flag(store: &RegistryStore, site: &str, remember: bool) -> Result<(), String> {
    let mut current = store
        .get(site)
        .map_err(|error| format!("Tresorliste lesen: {error}"))?
        .ok_or_else(|| "Tresorort ist nicht eingetragen".to_string())?;
    if current.remember_password == remember {
        return Ok(());
    }
    current.remember_password = remember;
    store.save(&current)
}

/// Stores `password` for the site, then sets the flag. A failed store
/// leaves the flag as it was (no success that did not happen).
pub(crate) fn remember(
    secrets: &dyn SecretStore,
    store: &RegistryStore,
    record: &SiteRecord,
    password: &str,
) -> Result<(), String> {
    secrets
        .set(&account(&record.id), password)
        .map_err(|error| format!("Passwort merken: {error}"))?;
    set_flag(store, &record.id, true)
}

/// Deletes the stored password of the site, then clears the flag.
pub(crate) fn forget(
    secrets: &dyn SecretStore,
    store: &RegistryStore,
    record: &SiteRecord,
) -> Result<(), String> {
    secrets
        .delete(&account(&record.id))
        .map_err(|error| format!("Gemerktes Passwort löschen: {error}"))?;
    set_flag(store, &record.id, false)
}

/// The stored password of `site`, if there is one.
pub(crate) fn saved_password(
    secrets: &dyn SecretStore,
    site: &str,
) -> Result<Option<Zeroizing<String>>, String> {
    secrets
        .get(&account(site))
        .map(|secret| secret.map(Zeroizing::new))
        .map_err(|error| format!("Gemerktes Passwort lesen: {error}"))
}

/// Applies the `remember` choice of an unlock that succeeded with
/// `password`: `Some(true)` stores, `Some(false)` deletes, `None` keeps.
/// The unlock stands either way; a failure comes back as text.
pub(crate) fn apply_choice(
    secrets: &dyn SecretStore,
    store: &RegistryStore,
    record: &SiteRecord,
    password: &str,
    choice: Option<bool>,
) -> Option<String> {
    let result = match choice {
        Some(true) => remember(secrets, store, record, password),
        Some(false) => forget(secrets, store, record),
        None => Ok(()),
    };
    result.err()
}
