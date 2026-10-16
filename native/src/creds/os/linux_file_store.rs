//! File credential store of Linux and Android (`secrets-v1/`).
//!
//! Every record is sealed with the store's data key (`sealed_record.rs`);
//! the key is held by the protector recorded in `keyring-v2`
//! (`key_protector.rs`). Plaintext records of earlier versions are ported
//! without any secret access when a worker, the app or `se doctor` starts
//! (`FileStore::port_existing`) and otherwise on the first access of each
//! process, then overwritten (`linux_file_store_migrate.rs`). Directory 0700,
//! files 0600, one `flock` for every access.
use std::path::PathBuf;
use std::sync::Arc;

use zeroize::Zeroizing;

#[path = "linux_file_store_fs.rs"]
mod fs;
#[path = "linux_file_store_keys.rs"]
mod keys;
#[path = "linux_file_store_migrate.rs"]
mod migrate;

use self::fs::{
    acquire_lock, open_directory, open_entry, open_existing_directory, read_entry, unlink_in,
    write_entry,
};
use crate::creds::key_protector::{KeyProtector, PlatformProtector};
use crate::creds::sealed_record::{
    account_digest, open_legacy, open_sealed, record_kind, record_name_for_digest, seal,
    RecordKind, MAX_RECORD_BYTES, MAX_SECRET_BYTES,
};

pub(super) struct FileStore {
    directory: PathBuf,
    protector: Arc<dyn KeyProtector>,
}

/// Where the store stands, for `se doctor`.
#[derive(Default)]
pub(super) struct StoreStatus {
    pub(super) protector: Option<String>,
    /// Plaintext records still waiting for the port.
    pub(super) plaintext_records: usize,
    /// Leftover stage files of interrupted writes.
    pub(super) stage_files: usize,
    /// Record files the store cannot inspect (see `migrate::Pending`).
    pub(super) unreadable_records: usize,
    /// Why the data key cannot be recovered (a descriptor exists, the
    /// protector delivers no key); `None` when the key opens or is not made yet.
    pub(super) locked: Option<String>,
}

impl FileStore {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self::with_protector(directory, Arc::new(PlatformProtector))
    }

    pub(super) fn with_protector(directory: PathBuf, protector: Arc<dyn KeyProtector>) -> Self {
        Self {
            directory,
            protector,
        }
    }

    pub(super) fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        validate_account(account)?;
        if secret.len() > MAX_SECRET_BYTES {
            return Err(format!(
                "Linux credential store: secret exceeds {MAX_SECRET_BYTES} bytes"
            ));
        }
        let directory = open_directory(&self.directory)?;
        let _lock = acquire_lock(&directory)?;
        // Porting other accounts never blocks this write (see `get`).
        let _ = self.port_once(&directory);
        let digest = account_digest(account);
        let name = record_name_for_digest(&digest);
        // An older version may have written this record in plaintext after
        // this process ported the store: its old bytes are overwritten once
        // the sealed record has replaced it.
        let replaced_plaintext = match open_entry(&directory, &name, true)? {
            Some(existing) => {
                fs::validate_record_file(&existing, "existing credential record")?;
                holds_plaintext(&existing).then_some(existing)
            }
            None => None,
        };
        let key = keys::load_or_create(&directory, &self.directory, self.protector.as_ref())?;
        let record = seal(&digest, secret.as_bytes(), &key)?;
        write_entry(&directory, &name, &record)?;
        if let Some(old) = replaced_plaintext {
            fs::scrub(&old);
        }
        Ok(())
    }

    pub(super) fn get(&self, account: &str) -> Result<Option<String>, String> {
        validate_account(account)?;
        let directory = open_directory(&self.directory)?;
        let _lock = acquire_lock(&directory)?;
        // A failed port never blocks reading: plaintext records stay
        // readable until the next access ports them.
        let _ = self.port_once(&directory);
        let digest = account_digest(account);
        let name = record_name_for_digest(&digest);
        let Some(file) = open_entry(&directory, &name, false)? else {
            return Ok(None);
        };
        let bytes = read_entry(&file, "credential record", MAX_RECORD_BYTES)?;
        let secret: Zeroizing<Vec<u8>> = match record_kind(&bytes)? {
            RecordKind::Sealed(_) => {
                let keys::StoreKey::Ready(key) =
                    keys::load(&directory, &self.directory, self.protector.as_ref())?
                else {
                    return Err(
                        "Geheimnisspeicher gesperrt: verschlüsselter Eintrag ohne Schlüsselbeschreibung"
                            .into(),
                    );
                };
                open_sealed(&digest, &bytes, &key)?
            }
            RecordKind::Legacy => {
                // Written by an older version after this process ported the
                // store: read it, then port it right away (best effort).
                let secret = open_legacy(&digest, &bytes)?;
                if let Ok(key) =
                    keys::load_or_create(&directory, &self.directory, self.protector.as_ref())
                {
                    let _ = migrate::port_record(&directory, &name, &digest, &key);
                }
                secret
            }
        };
        String::from_utf8(secret.to_vec())
            .map(Some)
            .map_err(|_| "Linux credential store: secret is not valid UTF-8".into())
    }

    pub(super) fn delete(&self, account: &str) -> Result<(), String> {
        validate_account(account)?;
        let directory = open_directory(&self.directory)?;
        let _lock = acquire_lock(&directory)?;
        let name = record_name_for_digest(&account_digest(account));
        let Some(file) = open_entry(&directory, &name, true)? else {
            return Ok(());
        };
        fs::validate_record_file(&file, "credential record")?;
        unlink_in(&directory, &name)?;
        fs::scrub(&file);
        directory
            .sync_all()
            .map_err(|error| fs::io_error("sync credential directory", error))
    }

    /// Protector in use, what still waits for the port, and whether the data
    /// key opens. Creates nothing.
    pub(super) fn status(&self) -> Result<StoreStatus, String> {
        let Some(directory) = open_existing_directory(&self.directory)? else {
            return Ok(StoreStatus::default());
        };
        let _lock = acquire_lock(&directory)?;
        let pending = migrate::scan(&directory, &self.directory)?;
        let protector = keys::protector_name(&directory)?;
        let locked = if protector.is_some() {
            keys::load(&directory, &self.directory, self.protector.as_ref()).err()
        } else {
            None
        };
        Ok(StoreStatus {
            protector,
            plaintext_records: pending.legacy.len(),
            stage_files: pending.stages.len(),
            unreadable_records: pending.unreadable,
            locked,
        })
    }

    /// Ports plaintext records and leftover stage files without reading any
    /// secret: at worker start, app start and `se doctor`, so nothing stays in
    /// plaintext on a host where no process ever reads a secret. Creates
    /// nothing when there is no store directory yet.
    pub(super) fn port_existing(&self) -> Result<(), String> {
        let Some(directory) = open_existing_directory(&self.directory)? else {
            return Ok(());
        };
        let _lock = acquire_lock(&directory)?;
        self.port_once(&directory)
    }

    /// Ports plaintext records once per process and directory; the process is
    /// marked only after everything succeeded, so a failure is retried at the
    /// next access. Leftover stage files go first and need no key; the data
    /// key is created only when there is a plaintext record to seal.
    fn port_once(&self, directory: &std::fs::File) -> Result<(), String> {
        if migrate::already_ported(&self.directory) {
            return Ok(());
        }
        let pending = migrate::scan(directory, &self.directory)?;
        let mut failed = migrate::clear_stages(directory, &pending)?;
        if !pending.legacy.is_empty() {
            let key = keys::load_or_create(directory, &self.directory, self.protector.as_ref())?;
            failed += migrate::port_legacy(directory, &pending, &key);
        }
        if failed > 0 {
            return Err(format!(
                "Linux credential store: {failed} plaintext record(s) or stage file(s) could not be ported yet"
            ));
        }
        migrate::mark_ported(&self.directory);
        Ok(())
    }

    #[cfg(test)]
    fn record_path(&self, account: &str) -> PathBuf {
        self.directory
            .join(record_name_for_digest(&account_digest(account)))
    }
}

/// Whether an open record file still holds the plaintext format 1.
fn holds_plaintext(file: &std::fs::File) -> bool {
    read_entry(file, "existing credential record", MAX_RECORD_BYTES)
        .is_ok_and(|bytes| matches!(record_kind(&bytes), Ok(RecordKind::Legacy)))
}

fn validate_account(account: &str) -> Result<(), String> {
    if account.is_empty() {
        Err("Linux credential store: account must not be empty".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "linux_file_store_tests.rs"]
mod tests;
