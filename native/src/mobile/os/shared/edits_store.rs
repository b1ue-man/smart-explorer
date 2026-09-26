//! The register of remote files opened for editing
//! (`<data>/mobile/edits.json`): where the local copy lives and the remote
//! and local state it was downloaded or last uploaded with.
use super::error::ApiError;
use super::runtime::{lock, Runtime};
use super::store::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Open remote copies kept at most.
pub(crate) const MAX_EDITS: usize = 100;

/// Missing fields load with their defaults, so a register written by another
/// app version stays readable.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct EditRecord {
    pub edit_id: String,
    pub name: String,
    pub location: String,
    pub local_path: String,
    /// Remote modification time at download or last upload (0 = unknown).
    pub remote_mtime_ms: i64,
    /// Local copy state at download or last upload.
    pub local_mtime_ms: i64,
    pub local_size: u64,
    /// A change was already announced with an `edits` event.
    pub notified: bool,
}

/// Modification time (ms) and size of a local file.
pub(crate) fn file_state(path: &Path) -> Option<(i64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_millis() as i64);
    Some((mtime, metadata.len()))
}

impl EditRecord {
    /// `None` when the local copy is gone, else whether it changed.
    pub(crate) fn modified(&self) -> Option<bool> {
        file_state(Path::new(&self.local_path))
            .map(|(mtime, size)| mtime != self.local_mtime_ms || size != self.local_size)
    }

    /// Adopts the current local state as the new baseline.
    pub(crate) fn rebase_local(&mut self) {
        self.rebase_to(file_state(Path::new(&self.local_path)));
    }

    /// Adopts `state` (see `file_state`) as the new local baseline.
    pub(crate) fn rebase_to(&mut self, state: Option<(i64, u64)>) {
        if let Some((mtime, size)) = state {
            self.local_mtime_ms = mtime;
            self.local_size = size;
        }
        self.notified = false;
    }
}

pub(crate) fn register_path(rt: &Runtime) -> PathBuf {
    rt.config().mobile_dir().join("edits.json")
}

/// `<cache>/open`, the root of all downloaded copies.
pub(crate) fn open_root(rt: &Runtime) -> PathBuf {
    rt.config().cache_subdir("open")
}

/// The register; a missing file is empty. An unreadable one is an error, so
/// it is never replaced and none of its copies is taken for an orphan.
pub(crate) fn load(rt: &Runtime) -> Result<Vec<EditRecord>, ApiError> {
    read_json::<Vec<EditRecord>>(&register_path(rt))
        .map_err(|error| ApiError::from(error).context("Geöffnete Dateien lesen"))
}

pub(crate) fn save(rt: &Runtime, records: &[EditRecord]) {
    if let Err(error) = write_json(&register_path(rt), &records) {
        rt.log_error("Geöffnete Dateien speichern", &error.to_string());
    }
}

/// Applies `change` to the register under its lock and saves it; an
/// unreadable register is left untouched and reported.
pub(crate) fn update<R>(
    rt: &Runtime,
    change: impl FnOnce(&mut Vec<EditRecord>) -> R,
) -> Result<R, ApiError> {
    let _serialized = lock(&rt.inner.edits_lock);
    let mut records = load(rt)?;
    let before = records.clone();
    let result = change(&mut records);
    if records != before {
        save(rt, &records);
    }
    Ok(result)
}

/// A random 16-hex-digit id for a new copy.
pub(crate) fn new_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        bytes = (nanos as u64 ^ u64::from(std::process::id())).to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
