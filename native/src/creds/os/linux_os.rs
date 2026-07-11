//! Linux owner-protected credential storage.
//!
//! The store intentionally has no Secret Service, D-Bus, desktop-session, or
//! kernel-keyring dependency. Linux filesystem ownership and mode bits are the
//! security boundary; encrypted home storage is required for offline secrecy.

#[path = "linux_file_store.rs"]
mod file_store;

use file_store::FileStore;

fn store() -> FileStore {
    FileStore::new(crate::support_dirs::app_data_dir().join("secrets-v1"))
}

pub(super) fn description() -> &'static str {
    "linux owner-protected files (0700/0600; no D-Bus)"
}

pub(super) fn set_secret(account: &str, secret: &str) -> Result<(), String> {
    store().set(account, secret)
}

pub(super) fn get_secret(account: &str) -> Result<Option<String>, String> {
    store().get(account)
}

pub(super) fn delete_secret(account: &str) -> Result<(), String> {
    store().delete(account)
}
