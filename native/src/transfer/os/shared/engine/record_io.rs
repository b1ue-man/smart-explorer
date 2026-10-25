//! Private atomic records; vault names never fall back to plaintext.
use super::view::Side;
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Read};
use std::path::Path;

pub(crate) type RecordContext = crate::vault::TransferBlobContext;

#[derive(Clone, Copy)]
pub(crate) enum RecordKind { Job, Alias }

impl RecordKind {
    fn purpose(self) -> crate::vault::TransferBlobPurpose {
        match self { Self::Job => crate::vault::TransferBlobPurpose::Job, Self::Alias => crate::vault::TransferBlobPurpose::Alias }
    }
}

pub(crate) fn record_locations(source: Side<'_>, target: Side<'_>, sources: &[&str], target_root: &str) -> io::Result<Vec<String>> {
    let mut locations = Vec::new();
    for (side, paths) in [(source, sources.to_vec()), (target, vec![target_root])] {
        match side {
            Side::Local => locations.extend(paths.into_iter().map(str::to_string)),
            Side::Remote(backend) => match crate::vault::vault_site_of(backend) {
                Some(Some(site)) => locations.push(format!("vault://{site}/")),
                Some(None) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Vaultzuordnung des Übertragungsauftrags ist unbestätigt")),
                None if backend.is_local() => locations.extend(paths.into_iter().map(str::to_string)),
                None => {},
            },
        }
    }
    Ok(locations)
}

pub(crate) fn record_context(locations: &[String]) -> io::Result<Option<RecordContext>> {
    crate::vault::transfer_blob_context(&locations.iter().map(String::as_str).collect::<Vec<_>>())
}

pub(crate) fn write_record<T: Serialize>(path: &Path, kind: RecordKind, id: &str, context: Option<&RecordContext>, value: &T) -> io::Result<()> {
    let plain = zeroize::Zeroizing::new(serde_json::to_vec(value).map_err(io::Error::other)?);
    match context {
        Some(context) => {
            let sealed = crate::vault::seal_transfer_blob(context, kind.purpose(), id, &plain)?;
            crate::support_dirs::write_private_atomic(path, &sealed)
        }
        None => crate::support_dirs::write_private_atomic(path, &plain),
    }
}

pub(crate) fn read_record<T: DeserializeOwned>(path: &Path, kind: RecordKind, id: &str, max_plain: u64, require_sealed: bool) -> io::Result<T> {
    read_record_state(path, kind, id, max_plain, require_sealed).map(|(value, _)| value)
}

pub(crate) fn read_record_state<T: DeserializeOwned>(path: &Path, kind: RecordKind, id: &str, max_plain: u64, require_sealed: bool) -> io::Result<(T, bool)> {
    let file = crate::support_dirs::open_private_file(path)?;
    let length = file.metadata().map_err(record_error)?.len();
    let bytes = usize::try_from(length).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "Jobdatensatz ist für diesen Prozess zu groß"))?;
    let _memory = crate::transfer::try_reserve_memory(length).ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "Nicht genügend freier Speicher für den Jobdatensatz"))?;
    let mut stored = zeroize::Zeroizing::new(Vec::with_capacity(bytes));
    file.take(length.saturating_add(1)).read_to_end(&mut stored).map_err(record_error)?;
    if stored.len() as u64 != length { return Err(io::Error::other("Jobdatensatz wurde während des Lesens geändert")); }
    let max_plain = max_plain.min(crate::transfer::memory_budget());
    match crate::vault::open_transfer_blob(kind.purpose(), id, &stored, usize::try_from(max_plain).unwrap_or(usize::MAX)) {
        Ok(plain) => serde_json::from_slice(&plain).map(|value| (value, true)).map_err(io::Error::other),
        Err(error) if require_sealed => Err(record_error(error)),
        Err(error) => {
            if stored.len() as u64 > max_plain { return Err(io::Error::new(io::ErrorKind::InvalidData, "Jobdatensatz überschreitet die zulässige Klartextgröße")); }
            // A sealed envelope cannot deserialize as the domain record.
            // In particular VaultLocked never admits an empty/default job.
            serde_json::from_slice(&stored).map(|value| (value, false)).map_err(|_| record_error(error))
        }
    }
}

fn record_error(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound { io::Error::new(io::ErrorKind::InvalidData, format!("Bestehender Jobdatensatz kann nicht bestätigt werden: {error}")) } else { error }
}

/// Runs in the catalog's loader thread, never in UI rendering. A failed
/// live observation drops cached plaintext until an explicit fresh reload.
pub(crate) fn monitor_contexts<T: Send + 'static>(state: std::sync::Weak<std::sync::Mutex<T>>, mut contexts: Vec<(String, RecordContext)>, revoke: impl Fn(&mut T, &str)) {
    while !contexts.is_empty() {
        let Some(shared) = state.upgrade() else { return; };
        contexts.retain(|(id, context)| {
            if crate::vault::check_transfer_blob_context_live(context).is_ok() { return true; }
            revoke(&mut shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner()), id);
            false
        });
        drop(shared);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
