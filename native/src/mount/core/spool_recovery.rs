//! A valid journal owns recovery data even when individual payloads are absent.
use super::{is_spool_name, validate_spool_name, PersistedEntry, WholeFileSpool};
use crate::mount::{EntryCondition, MountConflict};
use std::{fs, io, path::Path};

pub(super) fn require_journal_for_existing_files(root: &Path, files: &Path) -> io::Result<()> {
    for name in ["journal.jsonl", "journal.jsonl.compact-old", "journal.jsonl.compact-new"] {
        match fs::symlink_metadata(root.join(name)) {
            // Journal::open validates the object and recovers rotations itself.
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for child in fs::read_dir(files)? {
        let child = child?;
        if child.file_name().to_str().is_some_and(is_spool_name) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!(
                "Recovery-Journal fehlt in {}; vorhandene Cache-Dateien bleiben erhalten. Das Journal aus einer Sicherung wiederherstellen, bevor dieser Cache erneut verwendet wird",
                root.display())));
        }
    }
    Ok(())
}

pub(super) fn validate_entry(spool: &WholeFileSpool, entry: &mut PersistedEntry) -> io::Result<bool> {
    if entry.remote_path.is_empty() || entry.remote_path.contains('\0') {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "mount journal contains an invalid remote path"));
    }
    validate_spool_name(&entry.spool_name)?;
    match spool.open_file(&entry.spool_name, false) {
        Ok(file) => { file.metadata()?; Ok(false) }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Only the recovered view changes. Never forget/replace the durable
            // entry or substitute remote bytes for an absent local edit. Restoring
            // the payload and reopening recovers the original condition again.
            entry.condition = EntryCondition::Conflict(MountConflict {
                path: entry.remote_path.clone(),
                baseline: entry.baseline.clone(),
                current: None,
                detail: format!(
                    "Lokale Wiederherstellungsdatei fuer {} fehlt: {}. Journal und andere lokale Aenderungen bleiben erhalten; die fehlende Datei aus einer Sicherung wiederherstellen und das Laufwerk erneut verbinden",
                    entry.remote_path, spool.files.join(&entry.spool_name).display()),
            });
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn file_error(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("Mount-Cache-Datei {}: {error}", path.display()))
}
