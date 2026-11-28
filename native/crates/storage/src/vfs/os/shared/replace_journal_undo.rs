//! Undo of one replace run: every file of the run whose content is still
//! exactly what the replacement wrote (SHA-256) gets its original back,
//! published the same way (stage next to it, state checked right before,
//! `promote_staged`). A file changed since, removed, or no longer a regular
//! file stays as it is and is named with the reason; a later undo of the same
//! run tries those again. A file already back to its original counts as
//! restored.
use std::io;
use std::path::Path;

use super::content_replace_write::{publish, read_whole, PublishError};
use super::replace_journal::{
    backup_label, backup_name, load_entry, sha256_hex, store_entry, EntryRecord, ReplaceJournal,
    RunState, UndoSummary,
};
use super::replace_journal_seal::open_file;
use super::{Backend, BackendHandle, VfsResult};

const ENTRY_SUFFIX: &str = ".entry";

pub(super) fn undo(
    journal: &ReplaceJournal,
    id: &str,
    resolve: &dyn Fn(&str) -> VfsResult<BackendHandle>,
) -> VfsResult<UndoSummary> {
    let dir = journal.run_dir(id)?;
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("kein Ersetzungslauf {id}"),
        ));
    }
    let mut record = journal.read_record(&dir, id).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::new(io::ErrorKind::NotFound, format!("kein Ersetzungslauf {id}"))
        } else {
            error
        }
    })?;
    let backend = resolve(&record.location)?;
    let mut summary = UndoSummary::default();
    let mut open = 0usize;
    for loaded in entries(journal, &dir, id)? {
        let mut entry = match loaded {
            Ok(entry) => entry,
            Err((name, reason)) => {
                open += 1;
                summary.skipped.push((name, reason));
                continue;
            }
        };
        if entry.status == "restored" {
            continue;
        }
        match restore(journal, &*backend, (&dir, id), &entry) {
            Ok(listed) => {
                entry.status = "restored".to_string();
                if let Err(error) = store_entry(journal.key(), &dir, id, &entry) {
                    // Restored all the same; a later undo finds the original.
                    summary.skipped.push((
                        entry.rel.clone(),
                        format!("wiederhergestellt, Journal nicht aktualisiert: {error}"),
                    ));
                }
                if listed {
                    summary.restored.push(entry.rel);
                }
            }
            Err(reason) => {
                open += 1;
                summary.skipped.push((entry.rel, reason));
            }
        }
    }
    record.state = if open == 0 {
        RunState::Undone
    } else {
        RunState::PartlyUndone
    }
    .key()
    .to_string();
    journal.write_record(&dir, &record)?;
    Ok(summary)
}

type Loaded = Result<EntryRecord, (String, String)>;

/// The file records of a run in their order; one that cannot be read is
/// named with its record's file name.
fn entries(journal: &ReplaceJournal, dir: &Path, id: &str) -> VfsResult<Vec<Loaded>> {
    let mut found: Vec<(u64, Loaded)> = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let Some(index) = name
            .strip_suffix(ENTRY_SUFFIX)
            .and_then(|number| number.parse::<u64>().ok())
        else {
            continue;
        };
        let loaded = load_entry(journal.key(), &item.path(), id, index)
            .map_err(|error| (name.clone(), format!("Eintrag nicht lesbar: {error}")));
        found.push((index, loaded));
    }
    found.sort_by_key(|(index, _)| *index);
    Ok(found.into_iter().map(|(_, loaded)| loaded).collect())
}

/// Restores one file: `Ok(true)` restored (or found restored), `Ok(false)`
/// never published (nothing to do), `Err` the reason it stays.
fn restore(
    journal: &ReplaceJournal,
    backend: &dyn Backend,
    (dir, id): (&Path, &str),
    entry: &EntryRecord,
) -> Result<bool, String> {
    let meta = match backend.stat(&entry.path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err("nicht mehr vorhanden".to_string())
        }
        Err(error) => return Err(format!("nicht lesbar: {error}")),
    };
    if meta.is_dir || meta.is_symlink || meta.special {
        return Err("keine reguläre Datei mehr".to_string());
    }
    let current = read_whole(backend, &entry.path, &meta).map_err(|skip| skip.message())?;
    let current = sha256_hex(&current);
    if current == entry.original_sha256 {
        return Ok(entry.status != "pending");
    }
    if current != entry.new_sha256 {
        return Err("seit dem Ersetzen geändert".to_string());
    }
    let path = dir.join(backup_name(entry.index));
    let original = open_file(
        journal.key(),
        &backup_label(id, entry.index),
        &path,
        entry.original_size,
    )
    .map_err(|error| format!("Sicherung nicht lesbar: {error}"))?;
    if sha256_hex(&original) != entry.original_sha256 {
        return Err("Sicherung passt nicht zum Original".to_string());
    }
    match publish(backend, &entry.path, &original, &meta) {
        Ok(()) => Ok(true),
        Err(PublishError::Changed) => Err("während des Rückgängigmachens geändert".to_string()),
        Err(PublishError::Failed(error)) => Err(format!("Schreiben fehlgeschlagen: {error}")),
    }
}
