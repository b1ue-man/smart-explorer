//! Explicit secure-store limitation for non-Windows builds.
//!
//! The crate is intentionally built only with its Windows Credential Manager
//! backend. Its fallback mock is process-local and must never be reported as
//! durable storage.

fn unsupported() -> String {
    "Dauerhafte sichere Anmeldeinformationsspeicherung ist auf diesem Betriebssystem nicht konfiguriert"
        .to_string()
}

pub(super) fn set_secret(_account: &str, _secret: &str) -> Result<(), String> {
    Err(unsupported())
}

pub(super) fn get_secret(_account: &str) -> Result<Option<String>, String> {
    Err(unsupported())
}

pub(super) fn delete_secret(_account: &str) -> Result<(), String> {
    Err(unsupported())
}
