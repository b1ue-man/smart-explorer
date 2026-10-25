//! Pure, read-only journal inventory. Never opens a live Journal or a spool.
//! Randomized sealed paths cannot be used as replay keys without their key.
use super::super::journal_record::{apply, decode_record, Action, RecoveredJournal, MAX_RECORD_BYTES};
use super::super::types::EntryCondition;
use std::{collections::HashSet, io::{self, BufRead}};

pub(super) const MAX_AUDIT_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn pending(mut reader: impl BufRead) -> io::Result<Option<usize>> {
    let mut state = RecoveredJournal::default();
    let mut bytes = 0u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 { break; }
        bytes = bytes.saturating_add(read as u64);
        if bytes > MAX_AUDIT_BYTES || line.len() > MAX_RECORD_BYTES + 1 {
            return Err(invalid("Offlinejournal überschreitet seine unterstützte Größe; Originalbytes bleiben erhalten"));
        }
        if line.pop() != Some(b'\n') {
            // Only the authorized live opener may repair a torn journal tail.
            return Ok(None);
        }
        if line.is_empty() { continue; }
        let action = decode_record(&line)?;
        if !plain_paths(&action) { return Ok(None); }
        apply(&mut state, action);
    }
    let mut names = HashSet::new();
    for entry in state.entries.values() {
        super::validate_spool_name(&entry.spool_name)?;
        if !names.insert(&entry.spool_name) {
            return Err(invalid("Mehrere Journalzustände referenzieren dieselbe lokale Payload; Originalbytes bleiben erhalten"));
        }
        if entry.condition == EntryCondition::Clean && entry.delete_token.is_none() {
            return Err(invalid("Journalzustand ohne offene Änderung oder Löschtransaktion"));
        }
    }
    Ok(Some(state.entries.len().saturating_add(state.deletes.len())
        .saturating_add(state.namespace_conflicts.len())))
}

fn plain(path: &str) -> bool { path.starts_with('/') && !path.contains('\0') }

fn plain_paths(action: &Action) -> bool {
    match action {
        Action::UpsertEntry(entry) => plain(&entry.remote_path),
        Action::MoveEntry { old_path, entry } => plain(old_path) && plain(&entry.remote_path),
        Action::ForgetEntry { remote_path } => plain(remote_path),
        Action::UpsertDelete(delete) => plain(&delete.original_path) && plain(&delete.quarantine_path),
        Action::UpsertNamespaceConflict(intent) => plain(&intent.conflict.path)
            && intent.source_path.as_deref().is_none_or(plain),
        Action::ForgetNamespaceConflict { path } => plain(path),
        Action::ForgetDelete { .. } => true,
    }
}

fn invalid(detail: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, detail) }
