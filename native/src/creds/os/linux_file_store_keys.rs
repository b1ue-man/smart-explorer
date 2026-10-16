//! The data key of one store directory: its descriptor `keyring-v2`, created
//! once under the store lock, and a per-process cache so a protector such as
//! `systemd-creds` runs at most once per process and directory.
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::fs::{open_entry, read_entry, write_entry};
use crate::creds::key_protector::KeyProtector;
use crate::creds::sealed_record::{
    DataKey, KeyringDescriptor, DESCRIPTOR_FORMAT, DESCRIPTOR_NAME, MAX_DESCRIPTOR_BYTES,
};

static KEYS: Mutex<Option<HashMap<PathBuf, DataKey>>> = Mutex::new(None);

/// What a store directory says about its data key.
pub(super) enum StoreKey {
    /// The store has no data key yet (nothing was sealed so far).
    Missing,
    Ready(DataKey),
}

/// Loads the data key of `directory`; the caller holds the store lock.
pub(super) fn load(
    directory: &File,
    path: &Path,
    protector: &dyn KeyProtector,
) -> Result<StoreKey, String> {
    if let Some(key) = cached(path) {
        return Ok(StoreKey::Ready(key));
    }
    let Some(file) = open_entry(directory, DESCRIPTOR_NAME, false)? else {
        return Ok(StoreKey::Missing);
    };
    let bytes = read_entry(&file, "key descriptor", MAX_DESCRIPTOR_BYTES)?;
    let descriptor = KeyringDescriptor::parse(&bytes)?;
    let key = protector.recover(path, &descriptor)?;
    remember(path, &key);
    Ok(StoreKey::Ready(key))
}

/// Loads the data key, or creates, protects and durably records a new one.
pub(super) fn load_or_create(
    directory: &File,
    path: &Path,
    protector: &dyn KeyProtector,
) -> Result<DataKey, String> {
    if let StoreKey::Ready(key) = load(directory, path, protector)? {
        return Ok(key);
    }
    let key = protector.new_key()?;
    let protected = protector.protect(path, &key)?;
    let descriptor = KeyringDescriptor {
        format: DESCRIPTOR_FORMAT,
        protector: protected.protector.to_string(),
        key_id: key.id_hex(),
        wrapped: protected.wrapped,
    };
    let bytes = descriptor.encode()?;
    write_entry(directory, DESCRIPTOR_NAME, &bytes)?;
    // Read back through the protector: a descriptor that does not open again
    // must never seal a secret.
    let reread = protector.recover(path, &KeyringDescriptor::parse(&bytes)?)?;
    if reread.id() != key.id() {
        return Err("credential store: new data key does not open again".into());
    }
    remember(path, &key);
    Ok(key)
}

/// Name of the protector recorded for `directory`, for diagnostics.
pub(super) fn protector_name(directory: &File) -> Result<Option<String>, String> {
    let Some(file) = open_entry(directory, DESCRIPTOR_NAME, false)? else {
        return Ok(None);
    };
    let bytes = read_entry(&file, "key descriptor", MAX_DESCRIPTOR_BYTES)?;
    Ok(Some(KeyringDescriptor::parse(&bytes)?.protector))
}

fn cached(path: &Path) -> Option<DataKey> {
    KEYS.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(|keys| keys.get(path).cloned())
}

/// Tests: drops the cached key of `path`, as a new process would start
/// without it.
#[cfg(test)]
pub(super) fn forget(path: &Path) {
    if let Some(keys) = KEYS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        keys.remove(path);
    }
}

fn remember(path: &Path, key: &DataKey) {
    KEYS.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), key.clone());
}
