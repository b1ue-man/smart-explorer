//! Windows Credential Manager adapter.

use std::borrow::Cow;

use keyring::{Entry, Error};

#[cfg(debug_assertions)]
// The daemon adapter includes the same debug-only parser so both test-facing
// Windows resource names enforce one namespace contract without release code.
#[allow(clippy::duplicate_mod)]
#[path = "../../windows_test_namespace.rs"]
mod test_namespace;

const KEYRING_SERVICE: &str = "smart_explorer";
#[cfg(debug_assertions)]
const TEST_SERVICE_SEPARATOR: &str = ".test.";

pub(super) fn description() -> &'static str {
    "Windows Credential Manager"
}

fn entry(account: &str) -> Result<Entry, String> {
    let service = keyring_service_name()?;
    Entry::new(service.as_ref(), account)
        .map_err(|error| format!("Windows-Anmeldeinformationsverwaltung öffnen: {error}"))
}

#[cfg(not(debug_assertions))]
fn keyring_service_name() -> Result<Cow<'static, str>, String> {
    Ok(Cow::Borrowed(KEYRING_SERVICE))
}

#[cfg(debug_assertions)]
fn keyring_service_name() -> Result<Cow<'static, str>, String> {
    let namespace = test_namespace::from_env()?;
    keyring_service_name_for(namespace.as_deref()).map(Cow::Owned)
}

#[cfg(debug_assertions)]
fn keyring_service_name_for(namespace: Option<&str>) -> Result<String, String> {
    test_namespace::qualify(KEYRING_SERVICE, TEST_SERVICE_SEPARATOR, namespace)
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

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::keyring_service_name_for;

    #[test]
    fn credential_service_names_are_exact_and_isolated() {
        assert_eq!(keyring_service_name_for(None).unwrap(), "smart_explorer");
        assert_eq!(
            keyring_service_name_for(Some("device_A1")).unwrap(),
            "smart_explorer.test.device_A1"
        );
    }

    #[test]
    fn credential_service_rejects_an_unsafe_namespace() {
        assert!(keyring_service_name_for(Some(r"device\A")).is_err());
    }
}
