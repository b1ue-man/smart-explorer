//! Windows Credential Manager adapter.

use keyring::{Entry, Error};

const KEYRING_SERVICE: &str = "smart_explorer";

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|error| format!("Windows-Anmeldeinformationsverwaltung öffnen: {error}"))
}

pub(super) fn set_secret(account: &str, secret: &str) -> Result<(), String> {
    entry(account)?
        .set_password(secret)
        .map_err(|error| format!("Anmeldeinformation speichern: {error}"))
}

pub(super) fn get_secret(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("Anmeldeinformation lesen: {error}")),
    }
}

pub(super) fn delete_secret(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Anmeldeinformation löschen: {error}")),
    }
}
