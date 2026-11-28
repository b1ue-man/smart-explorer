//! Side records of the mask store in the se secret store: the store key,
//! a marker that a store exists (so deleting both files is noticed), the
//! last configuration generation (an older copy of the file is refused) and
//! the lockdown flag (a changed `state.json` alone does not lift it).
//! Stores opened with `MaskStore::open_at` keep these records in memory per
//! directory, so tests of several "processes" share them.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

pub(crate) const KEY: &str = "smart-explorer:masks:key";
pub(crate) const MARKER: &str = "smart-explorer:masks:present";
pub(crate) const GENERATION: &str = "smart-explorer:masks:config-generation";
pub(crate) const LOCKDOWN: &str = "smart-explorer:masks:lockdown";

static MEMORY: Mutex<BTreeMap<(PathBuf, &'static str), String>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Debug)]
pub(crate) enum Side {
    /// The se secret store (Credential Manager, sealed file store).
    Secrets,
    /// In memory, keyed by the store directory.
    Memory(PathBuf),
}

impl Side {
    pub(crate) fn get(&self, account: &'static str) -> Result<Option<String>, String> {
        match self {
            Self::Secrets => crate::creds::get_secret_checked(account),
            Self::Memory(dir) => Ok(memory().get(&(dir.clone(), account)).cloned()),
        }
    }

    pub(crate) fn set(&self, account: &'static str, value: &str) -> Result<(), String> {
        match self {
            // Not a login credential: no "credentials changed" signal.
            Self::Secrets => crate::creds::set_secret_untracked(account, value),
            Self::Memory(dir) => {
                memory().insert((dir.clone(), account), value.to_string());
                Ok(())
            }
        }
    }

    pub(crate) fn delete(&self, account: &'static str) -> Result<(), String> {
        match self {
            Self::Secrets => crate::creds::delete_secret_checked(account),
            Self::Memory(dir) => {
                memory().remove(&(dir.clone(), account));
                Ok(())
            }
        }
    }

    pub(crate) fn generation(&self) -> Result<u64, String> {
        Ok(self
            .get(GENERATION)?
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0))
    }
}

fn memory() -> std::sync::MutexGuard<'static, BTreeMap<(PathBuf, &'static str), String>> {
    MEMORY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Test helper: forget the memory records of `dir`.
#[cfg(test)]
pub(crate) fn forget(dir: &std::path::Path) {
    memory().retain(|(owner, _), _| owner != dir);
}
