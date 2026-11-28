//! The replace journal (plan docs/plaene/2026-11-28-ersetzen-masken-shell,
//! part C „Sicherung“): before a replacement overwrites a file, its original
//! is kept encrypted in the app data (`<app data>/replace-journal/<run>/`,
//! key in the secret store, `replace_journal_seal.rs`). Every run lists its
//! files; `undo` restores a run's files that are unchanged since the
//! replacement (others are named with the reason), `discard` deletes a run.
//! The original survives a crash between backup and publication: a file's
//! record is written before its new content is published.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::replace_journal_seal::{open_file, seal_new_file, seal_replace_file, JournalKey};
use super::{BackendHandle, VfsResult};
use crate::agent_proto::{ContentQuery, TextMode};

/// The secret-store account of the journal key.
const KEY_ACCOUNT: &str = "replace:journal:key";
const HEADER_NAME: &str = "run.sealed";
/// Largest record (run or file) the journal reads.
const RECORD_LIMIT: u64 = 1024 * 1024;

/// The state of a run in the journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    /// Still running, or ended without its last record (a crash).
    Running,
    Applied,
    /// `undo` restored some files; others had changed since.
    PartlyUndone,
    Undone,
}

impl RunState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Applied => "applied",
            Self::PartlyUndone => "partly_undone",
            Self::Undone => "undone",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "applied" => Self::Applied,
            "partly_undone" => Self::PartlyUndone,
            "undone" => Self::Undone,
            _ => Self::Running,
        }
    }
}

/// One run as `history` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceRun {
    pub id: String,
    /// Unix milliseconds of the start.
    pub created_ms: i64,
    /// The place as the surface named it (se target grammar), for `undo`.
    pub location: String,
    /// The root folder in the backend of `location`.
    pub root: String,
    pub pattern: String,
    pub replacement: String,
    pub mode: TextMode,
    pub files_changed: u64,
    pub replacements: u64,
    pub state: RunState,
}

/// What `undo` did: restored files and files left as they are, with the reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoSummary {
    /// Relative paths of the files that have their original content again.
    pub restored: Vec<String>,
    /// (relative path, German reason).
    pub skipped: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RunRecord {
    pub(super) id: String,
    pub(super) created_ms: i64,
    pub(super) location: String,
    pub(super) root: String,
    pub(super) pattern: String,
    pub(super) replacement: String,
    pub(super) mode: String,
    pub(super) state: String,
    pub(super) files_changed: u64,
    pub(super) replacements: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EntryRecord {
    pub(super) index: u64,
    pub(super) rel: String,
    /// The file in the backend of the run's location.
    pub(super) path: String,
    pub(super) original_size: u64,
    pub(super) original_sha256: String,
    pub(super) new_sha256: String,
    pub(super) replacements: u64,
    /// `pending` (before publication), `applied`, `restored`.
    pub(super) status: String,
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn io_error(message: String) -> io::Error {
    io::Error::other(message)
}

/// The journal of replacements of this user.
pub struct ReplaceJournal {
    dir: PathBuf,
    key: JournalKey,
}

impl std::fmt::Debug for ReplaceJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplaceJournal")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl ReplaceJournal {
    /// The journal in the app data, its key in the secret store (created on
    /// first use).
    pub fn open_default() -> VfsResult<Self> {
        let dir = crate::support_dirs::app_data_dir().join("replace-journal");
        crate::support_dirs::ensure_private_dir(&dir)?;
        Ok(Self {
            dir,
            key: default_key()?,
        })
    }

    /// A journal in `dir` with `key` (tests, other stores).
    pub fn open_at(dir: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        Self {
            dir: dir.into(),
            key: JournalKey::from_bytes(key),
        }
    }

    pub(super) fn key(&self) -> &JournalKey {
        &self.key
    }

    pub(super) fn run_dir(&self, id: &str) -> VfsResult<PathBuf> {
        let valid = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if !valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("ungültige Kennung eines Ersetzungslaufs: {id}"),
            ));
        }
        Ok(self.dir.join(id))
    }

    pub(super) fn read_record(&self, dir: &Path, id: &str) -> VfsResult<RunRecord> {
        let plain = open_file(
            &self.key,
            &header_label(id),
            &dir.join(HEADER_NAME),
            RECORD_LIMIT,
        )?;
        serde_json::from_slice(&plain)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }

    pub(super) fn write_record(&self, dir: &Path, record: &RunRecord) -> VfsResult<()> {
        let plain = serde_json::to_vec(record).map_err(|error| io_error(error.to_string()))?;
        seal_replace_file(
            &self.key,
            &header_label(&record.id),
            &plain,
            &dir.join(HEADER_NAME),
        )
    }

    /// The runs, newest first; a run whose record cannot be read is left out.
    pub fn history(&self) -> VfsResult<Vec<ReplaceRun>> {
        let listing = match std::fs::read_dir(&self.dir) {
            Ok(listing) => listing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut runs = Vec::new();
        for item in listing {
            let item = item?;
            let Some(id) = item.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Ok(dir) = self.run_dir(&id) else {
                continue;
            };
            if let Ok(record) = self.read_record(&dir, &id) {
                runs.push(ReplaceRun {
                    mode: TextMode::parse(&record.mode).unwrap_or_default(),
                    state: RunState::parse(&record.state),
                    id: record.id,
                    created_ms: record.created_ms,
                    location: record.location,
                    root: record.root,
                    pattern: record.pattern,
                    replacement: record.replacement,
                    files_changed: record.files_changed,
                    replacements: record.replacements,
                });
            }
        }
        runs.sort_by(|left, right| {
            right
                .created_ms
                .cmp(&left.created_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(runs)
    }

    /// Restores the files of run `id` that are unchanged since it replaced
    /// them; `resolve` opens the run's location.
    pub fn undo(
        &self,
        id: &str,
        resolve: &dyn Fn(&str) -> VfsResult<BackendHandle>,
    ) -> VfsResult<UndoSummary> {
        super::replace_journal_undo::undo(self, id, resolve)
    }

    /// Deletes run `id` with its encrypted originals.
    pub fn discard(&self, id: &str) -> VfsResult<()> {
        let dir = self.run_dir(id)?;
        match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("kein Ersetzungslauf {id}"),
            )),
            other => other,
        }
    }

    /// Starts the record of a new run.
    pub(super) fn begin(
        &self,
        location: &str,
        root: &str,
        query: &ContentQuery,
        replacement: &str,
    ) -> VfsResult<RunWriter> {
        crate::support_dirs::ensure_private_dir(&self.dir)?;
        let mut suffix = [0u8; 4];
        getrandom::getrandom(&mut suffix).map_err(|error| io_error(error.to_string()))?;
        let id = format!(
            "{}-{:08x}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            u32::from_le_bytes(suffix)
        );
        let dir = self.run_dir(&id)?;
        crate::support_dirs::ensure_private_dir(&dir)?;
        let record = RunRecord {
            id: id.clone(),
            created_ms: now_ms(),
            location: location.to_string(),
            root: root.to_string(),
            pattern: query.pattern.clone(),
            replacement: replacement.to_string(),
            mode: query.mode.key().to_string(),
            state: RunState::Running.key().to_string(),
            files_changed: 0,
            replacements: 0,
        };
        self.write_record(&dir, &record)?;
        Ok(RunWriter {
            key: self.key.clone(),
            dir,
            record: Mutex::new(record),
            next: AtomicU64::new(0),
            applied: AtomicU64::new(0),
        })
    }
}

/// Writes the record of one file of run `id` (in `dir`).
pub(super) fn store_entry(
    key: &JournalKey,
    dir: &Path,
    id: &str,
    record: &EntryRecord,
) -> io::Result<()> {
    let plain = serde_json::to_vec(record).map_err(|error| io_error(error.to_string()))?;
    let path = dir.join(entry_name(record.index));
    seal_replace_file(key, &entry_label(id, record.index), &plain, &path)
}

/// Reads the record of one file of run `id` from `path`.
pub(super) fn load_entry(
    key: &JournalKey,
    path: &Path,
    id: &str,
    index: u64,
) -> io::Result<EntryRecord> {
    let plain = open_file(key, &entry_label(id, index), path, RECORD_LIMIT)?;
    serde_json::from_slice(&plain)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

pub(super) fn header_label(id: &str) -> String {
    format!("{id}/run")
}

pub(super) fn entry_label(id: &str, index: u64) -> String {
    format!("{id}/{index}/entry")
}

pub(super) fn backup_label(id: &str, index: u64) -> String {
    format!("{id}/{index}/backup")
}

pub(super) fn entry_name(index: u64) -> String {
    format!("{index:08}.entry")
}

pub(super) fn backup_name(index: u64) -> String {
    format!("{index:08}.backup")
}

/// The key of the default journal: read from the secret store, created there
/// on first use (the stored one is used, so two processes agree on it).
fn default_key() -> VfsResult<JournalKey> {
    let read = || {
        crate::creds::get_secret_checked(KEY_ACCOUNT)
            .map_err(|error| io_error(format!("Geheimnisspeicher nicht lesbar: {error}")))
    };
    if let Some(stored) = read()? {
        return JournalKey::from_hex(&stored);
    }
    let fresh = JournalKey::generate()?;
    crate::creds::set_secret_untracked(KEY_ACCOUNT, &fresh.to_hex())
        .map_err(|error| io_error(format!("Geheimnisspeicher nicht beschreibbar: {error}")))?;
    match read()? {
        Some(stored) => JournalKey::from_hex(&stored),
        None => Err(io_error(
            "der Schlüssel des Ersetzungsjournals wurde nicht gespeichert".to_string(),
        )),
    }
}

/// The record of one running replacement; shared by its threads.
pub(super) struct RunWriter {
    key: JournalKey,
    dir: PathBuf,
    record: Mutex<RunRecord>,
    next: AtomicU64,
    applied: AtomicU64,
}

/// A file whose original is kept, before and after its publication.
pub(super) struct JournalEntry {
    record: EntryRecord,
}

impl RunWriter {
    pub(super) fn id(&self) -> String {
        self.record
            .lock()
            .map(|record| record.id.clone())
            .unwrap_or_default()
    }

    /// Keeps `original` of `rel` (at `path`) encrypted and records the file
    /// as pending; `Err`: nothing is kept, the file must stay unchanged.
    pub(super) fn backup(
        &self,
        (rel, path): (&str, &str),
        original: &[u8],
        new: &[u8],
        replacements: u64,
    ) -> io::Result<JournalEntry> {
        let id = self.id();
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let record = EntryRecord {
            index,
            rel: rel.to_string(),
            path: path.to_string(),
            original_size: original.len() as u64,
            original_sha256: sha256_hex(original),
            new_sha256: sha256_hex(new),
            replacements,
            status: "pending".to_string(),
        };
        let backup = self.dir.join(backup_name(index));
        seal_new_file(&self.key, &backup_label(&id, index), original, &backup)?;
        let entry = JournalEntry { record };
        if let Err(error) = self.write_entry(&id, &entry.record) {
            let _ = std::fs::remove_file(&backup);
            return Err(error);
        }
        Ok(entry)
    }

    fn write_entry(&self, id: &str, record: &EntryRecord) -> io::Result<()> {
        store_entry(&self.key, &self.dir, id, record)
    }

    /// The file's new content is published.
    pub(super) fn applied(&self, mut entry: JournalEntry) {
        self.applied.fetch_add(1, Ordering::Relaxed);
        entry.record.status = "applied".to_string();
        // A record left `pending` is restored all the same (by its hashes).
        let _ = self.write_entry(&self.id(), &entry.record);
    }

    /// The file stayed unchanged: its backup and record go.
    pub(super) fn abandon(&self, entry: JournalEntry) {
        let _ = std::fs::remove_file(self.dir.join(entry_name(entry.record.index)));
        let _ = std::fs::remove_file(self.dir.join(backup_name(entry.record.index)));
    }

    /// Ends the run: its totals and state, or (nothing published) no run at
    /// all. Returns the run's id when it stays.
    pub(super) fn finish(&self, files_changed: u64, replacements: u64) -> Option<String> {
        let id = self.id();
        if self.applied.load(Ordering::Relaxed) == 0 {
            let _ = std::fs::remove_dir_all(&self.dir);
            return None;
        }
        let record = self.record.lock().ok().map(|mut record| {
            record.files_changed = files_changed;
            record.replacements = replacements;
            record.state = RunState::Applied.key().to_string();
            record.clone()
        });
        if let Some(record) = record {
            let plain = serde_json::to_vec(&record).unwrap_or_default();
            let path = self.dir.join(HEADER_NAME);
            let _ = seal_replace_file(&self.key, &header_label(&id), &plain, &path);
        }
        Some(id)
    }
}
