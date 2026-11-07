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
        // Text rows: the text starts with `prefix`, contains every `present`
        // fragment and no `absent` fragment. The app-key-file row explains
        // what the key file protects.
        let texts: [(&str, StoreStatus, &str, &[&str], &[&str]); 2] = [
            (
                "systemd-creds-user",
                StoreStatus {
                    protector: Some("systemd-creds-user".into()),
                    plaintext_records: 2,
                    stage_files: 1,
                    unreadable_records: 3,
                    locked: None,
                },
                "protector=systemd-creds-user plaintext_records=2 stage_files=1 unreadable_records=3",
                &[],
                &["disk encryption"],
            ),
            (
                "app-key-file",
                StoreStatus {
                    protector: Some("app-key-file".into()),
                    ..StoreStatus::default()
                },
                "protector=app-key-file",
                &["plaintext_records=0", "disk encryption"],
                &[],
            ),
        ];
        for (case, status, prefix, present, absent) in texts {
            let text = describe(status).unwrap();
            assert!(text.starts_with(prefix), "{case}: {text}");
            for fragment in present {
                assert!(text.contains(fragment), "{case}: {text}");
            }
            for fragment in absent {
                assert!(!text.contains(fragment), "{case}: {text}");
            }
        }

        // A locked store is an error that names the reason and keeps the counts.
        let locked = [(
            "locked",
            StoreStatus {
                protector: Some("systemd-creds-user".into()),
                locked: Some("systemd-creds antwortet nicht".into()),
                ..StoreStatus::default()
            },
            &[
                "locked",
                "systemd-creds antwortet nicht",
                "plaintext_records=0",
            ],
        )];
        for (case, status, fragments) in locked {
            let error = describe(status).unwrap_err();
            for fragment in fragments {
                assert!(error.contains(fragment), "{case}: {error}");
            }
        }
    }
}
