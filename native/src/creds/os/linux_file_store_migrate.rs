//! Porting of format-1 plaintext records to sealed format-2 records.
//!
//! Runs under the store lock. Each record is sealed into a stage file,
//! synced, read back and opened, and only then renamed over the plaintext
//! record; the replaced plaintext is then overwritten through the handle that
//! was still open. A crash leaves either the old or the new complete record.
//! Stage files left by an interrupted earlier write are scrubbed and removed;
//! they need no data key, so even a locked store loses them.
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::fs::{open_entry, read_entry, scrub, unlink_in, write_entry, STAGE_PREFIX};
use crate::creds::sealed_record::{
    digest_from_record_name, open_legacy, open_sealed, record_kind, seal, DataKey, RecordKind,
    ACCOUNT_DIGEST_BYTES, MAX_RECORD_BYTES,
};

/// Directories this process has already ported once.
static PORTED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

pub(super) fn already_ported(path: &Path) -> bool {
    PORTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .is_some_and(|ported| ported.contains(path))
}

pub(super) fn mark_ported(path: &Path) {
    PORTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashSet::new)
        .insert(path.to_path_buf());
}

/// Tests: makes the next access port again, as a new process would.
#[cfg(test)]
pub(super) fn forget_ported(path: &Path) {
    if let Some(ported) = PORTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        ported.remove(path);
    }
}

/// What the store directory still holds that needs porting or attention.
pub(super) struct Pending {
    /// Plaintext (format 1) records.
    pub(super) legacy: Vec<(String, [u8; ACCOUNT_DIGEST_BYTES])>,
    /// Leftover stage files of interrupted writes (older versions staged
    /// plaintext).
    pub(super) stages: Vec<String>,
    /// Record files that cannot be inspected (wrong mode, hard link, damaged
    /// or oversized). They are never touched, but `se doctor` shows them: one
    /// of them could still hold an old plaintext secret.
    pub(super) unreadable: usize,
}

/// Lists what still needs porting. Only exact record names and stage names
/// are considered; every other entry is left alone.
pub(super) fn scan(directory: &File, path: &Path) -> Result<Pending, String> {
    let mut pending = Pending {
        legacy: Vec::new(),
        stages: Vec::new(),
        unreadable: 0,
    };
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("Linux credential store: list credential directory: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("Linux credential store: list credential directory: {error}")
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with(STAGE_PREFIX) {
            pending.stages.push(name);
            continue;
        }
        let Some(digest) = digest_from_record_name(&name) else {
            continue;
        };
        // An unreadable or damaged record keeps its own error for its own
        // account; it never blocks porting the others.
        let Ok(Some(file)) = open_entry(directory, &name, false) else {
            pending.unreadable += 1;
            continue;
        };
        let Ok(bytes) = read_entry(&file, "credential record", MAX_RECORD_BYTES) else {
            pending.unreadable += 1;
            continue;
        };
        if matches!(record_kind(&bytes), Ok(RecordKind::Legacy)) {
            pending.legacy.push((name, digest));
        }
    }
    Ok(pending)
}

/// Scrubs and removes the leftover stage files. Needs no data key. Returns
/// how many could not be removed.
pub(super) fn clear_stages(directory: &File, pending: &Pending) -> Result<usize, String> {
    let mut failed = 0;
    for name in &pending.stages {
        // Only a private regular stage is scrubbed; a link is just unlinked.
        if let Ok(Some(file)) = open_entry(directory, name, true) {
            if super::fs::validate_secure_file(&file, "staged record").is_ok() {
                scrub(&file);
            }
        }
        if unlink_in(directory, name).is_err() {
            failed += 1;
        }
    }
    if !pending.stages.is_empty() {
        directory.sync_all().map_err(|error| {
            format!("Linux credential store: sync credential directory: {error}")
        })?;
    }
    Ok(failed)
}

/// Seals every plaintext record. Returns how many could not be sealed yet;
/// each failure leaves its plaintext record untouched for the next attempt.
pub(super) fn port_legacy(directory: &File, pending: &Pending, key: &DataKey) -> usize {
    pending
        .legacy
        .iter()
        .filter(|(name, digest)| port_record(directory, name, digest, key).is_err())
        .count()
}

/// Seals one plaintext record in place. `Ok(false)` when it is already sealed.
pub(super) fn port_record(
    directory: &File,
    name: &str,
    digest: &[u8; ACCOUNT_DIGEST_BYTES],
    key: &DataKey,
) -> Result<bool, String> {
    let Some(plain_file) = open_entry(directory, name, true)? else {
        return Ok(false);
    };
    let bytes = read_entry(&plain_file, "credential record", MAX_RECORD_BYTES)?;
    if record_kind(&bytes)? != RecordKind::Legacy {
        return Ok(false);
    }
    let secret = open_legacy(digest, &bytes)?;
    let sealed = seal(digest, &secret, key)?;
    if open_sealed(digest, &sealed, key)?.as_slice() != secret.as_slice() {
        return Err(
            "Linux credential store: sealed record does not open to the same secret".into(),
        );
    }
    write_entry(directory, name, &sealed)?;
    let stored = open_entry(directory, name, false)?
        .ok_or_else(|| "Linux credential store: sealed record vanished".to_string())?;
    let stored = read_entry(&stored, "credential record", MAX_RECORD_BYTES)?;
    if open_sealed(digest, &stored, key)?.as_slice() != secret.as_slice() {
        return Err("Linux credential store: stored sealed record does not open".into());
    }
    scrub(&plain_file);
    Ok(true)
}
