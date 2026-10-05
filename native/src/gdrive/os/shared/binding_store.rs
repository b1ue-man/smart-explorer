//! Private, process- and file-locked folder identity transactions. Network
//! evidence is gathered by the provider before entering this host boundary.
use super::sync_bindings::FolderBindings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static TRANSACTION: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DiskBindings {
    checksum: String,
    bindings: FolderBindings,
}

pub(super) struct BindingStore {
    root: Option<PathBuf>,
    memory: Mutex<HashMap<(String, String), FolderBindings>>,
}

impl BindingStore {
    pub(super) fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            memory: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn read(&self, account: &str, parent: &str) -> io::Result<FolderBindings> {
        self.transact(account, parent, |record| Ok(record.clone()))
    }

    pub(super) fn transact<T>(
        &self,
        account: &str,
        parent: &str,
        edit: impl FnOnce(&mut FolderBindings) -> io::Result<T>,
    ) -> io::Result<T> {
        let _process = TRANSACTION
            .lock()
            .map_err(|_| io::Error::other("Drive binding transaction poisoned"))?;
        if let Some(root) = &self.root {
            let directory = root.join(digest(account));
            crate::support_dirs::ensure_private_dir(&directory)?;
            let stem = digest(parent);
            let lock =
                crate::support_dirs::open_private_lock(&directory.join(format!("{stem}.lock")))?;
            lock.lock()?;
            let path = directory.join(format!("{stem}.json"));
            let mut record = load(&path, account, parent)?;
            let before = record.clone();
            let result = edit(&mut record)?;
            record.validate(account, parent)?;
            if before != record {
                let bytes = serde_json::to_vec(&DiskBindings {
                    checksum: record_checksum(&record)?,
                    bindings: record.clone(),
                })
                .map_err(io::Error::other)?;
                crate::support_dirs::write_private_atomic(&path, &bytes)?;
            }
            self.memory
                .lock()
                .map_err(|_| io::Error::other("Drive binding memory poisoned"))?
                .insert((account.to_string(), parent.to_string()), record);
            // Closing the separate private lock handle releases the lock.
            return Ok(result);
        }
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| io::Error::other("Drive binding memory poisoned"))?;
        let key = (account.to_string(), parent.to_string());
        let mut record = memory
            .get(&key)
            .cloned()
            .unwrap_or_else(|| FolderBindings::empty(account, parent));
        record.validate(account, parent)?;
        let result = edit(&mut record)?;
        record.validate(account, parent)?;
        memory.insert(key, record);
        Ok(result)
    }
}

fn load(path: &Path, account: &str, parent: &str) -> io::Result<FolderBindings> {
    let file = match crate::support_dirs::open_private_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FolderBindings::empty(account, parent))
        }
        Err(error) => return Err(error),
    };
    let disk: DiskBindings = serde_json::from_reader(file).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Drive folder bindings are unreadable; preserving the existing record: {error}"
            ),
        )
    })?;
    let record = disk.bindings;
    record.validate(account, parent)?;
    if record_checksum(&record)? != disk.checksum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Drive folder bindings checksum differs; preserving the existing record",
        ));
    }
    Ok(record)
}

fn record_checksum(record: &FolderBindings) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) fn record_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("gdrive")
        .join("sync-bindings")
}

pub(super) fn account_cache_path(account: &str) -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("gdrive")
        .join("accounts")
        .join(digest(account))
        .join("path_cache.json")
}

pub(super) fn legacy_cache_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("gdrive")
        .join("path_cache.json")
}

pub(super) fn read_hint_cache(path: &Path) -> io::Result<String> {
    let mut file = crate::support_dirs::open_private_file(path)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

pub(super) fn write_hint_cache(path: &Path, bytes: &[u8]) -> io::Result<()> {
    crate::support_dirs::write_private_atomic(path, bytes)
}

fn digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
