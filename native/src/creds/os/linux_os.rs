//! Linux and Android credential storage: owner-protected files (0700/0600)
//! whose records are sealed with a data key held by the protector recorded
//! in the store (`key_protector.rs`). No Secret Service or session keyring:
//! the store must open for headless workers and boot-time services.

#[path = "linux_file_store.rs"]
mod file_store;

use crate::creds::key_protector::APP_KEY_FILE;
use file_store::FileStore;

fn store() -> FileStore {
    FileStore::new(crate::support_dirs::app_data_dir().join("secrets-v1"))
}

pub(super) fn description() -> &'static str {
    "owner-protected sealed files (0700/0600, ChaCha20-Poly1305, no D-Bus)"
}

/// Protector of the data key, what still waits for the port, and whether the
/// key opens (an `Err` when the store is locked).
pub(super) fn status() -> Option<Result<String, String>> {
    Some(store().status().and_then(describe))
}

fn describe(status: file_store::StoreStatus) -> Result<String, String> {
    let protector = status.protector.as_deref().unwrap_or("none-yet");
    let mut text = format!(
        "protector={protector} plaintext_records={} stage_files={} unreadable_records={}",
        status.plaintext_records, status.stage_files, status.unreadable_records
    );
    if protector == APP_KEY_FILE {
        text.push_str(
            "; app-key-file: the key file lies beside the store, so only a copy of the store directory alone stays closed; protection of the whole data directory against offline access needs disk encryption",
        );
    }
    match status.locked {
        Some(reason) => Err(format!("credential store is locked: {reason}; {text}")),
        None => Ok(text),
    }
}

/// Seals plaintext records of earlier versions without any secret access.
pub(super) fn port_store() -> Result<(), String> {
    store().port_existing()
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

#[cfg(test)]
mod tests {
    use super::describe;
    use super::file_store::StoreStatus;

    #[test]
    fn luecken_task_status_text_names_the_protector_and_every_count() {
        let text = describe(StoreStatus {
            protector: Some("systemd-creds-user".into()),
            plaintext_records: 2,
            stage_files: 1,
            unreadable_records: 3,
            locked: None,
        })
        .unwrap();
        assert!(text.starts_with(
            "protector=systemd-creds-user plaintext_records=2 stage_files=1 unreadable_records=3"
        ));
        assert!(!text.contains("disk encryption"));
    }

    #[test]
    fn luecken_task_status_text_explains_what_app_key_file_protects() {
        let text = describe(StoreStatus {
            protector: Some("app-key-file".into()),
            ..StoreStatus::default()
        })
        .unwrap();
        assert!(text.contains("plaintext_records=0"));
        assert!(text.contains("disk encryption"));
    }

    #[test]
    fn luecken_task_status_of_a_locked_store_is_an_error_with_its_reason() {
        let error = describe(StoreStatus {
            protector: Some("systemd-creds-user".into()),
            locked: Some("systemd-creds antwortet nicht".into()),
            ..StoreStatus::default()
        })
        .unwrap_err();
        assert!(error.contains("locked"));
        assert!(error.contains("systemd-creds antwortet nicht"));
        assert!(error.contains("plaintext_records=0"));
    }
}
