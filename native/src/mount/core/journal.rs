use super::types::{Baseline, EntryCondition, NamespaceIntent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const JOURNAL_VERSION: u8 = 1;
const COMPACT_AT_BYTES: u64 = 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum TornTail {
    Truncate,
    Reject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PersistedEntry {
    pub remote_path: String,
    pub spool_name: String,
    pub baseline: Baseline,
    pub condition: EntryCondition,
    #[serde(default)]
    pub delete_token: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum DeletePhase {
    LocalOnly,
    /// A dispatched quarantine move has no restart-safe commit/rollback inference.
    Unresolved,
    /// Legacy journal value; recovery treats it like `Unresolved`.
    Prepared,
    Moved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PersistedDelete {
    pub token: u64,
    pub original_path: String,
    pub quarantine_path: String,
    pub id: Option<String>,
    pub is_directory: bool,
    pub phase: DeletePhase,
}

#[derive(Clone, Default)]
pub(super) struct RecoveredJournal {
    pub entries: HashMap<String, PersistedEntry>,
    pub deletes: HashMap<u64, PersistedDelete>,
    pub namespace_conflicts: HashMap<String, NamespaceIntent>,
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u8,
    action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
enum Action {
    UpsertEntry(PersistedEntry),
    MoveEntry {
        old_path: String,
        entry: PersistedEntry,
    },
    ForgetEntry {
        remote_path: String,
    },
    UpsertDelete(PersistedDelete),
    UpsertNamespaceConflict(NamespaceIntent),
    ForgetNamespaceConflict {
        path: String,
    },
    ForgetDelete {
        token: u64,
    },
}

pub(super) struct Journal {
    path: PathBuf,
    file: Option<File>,
    state: RecoveredJournal,
}

impl Journal {
    pub fn open(path: &Path) -> io::Result<(Self, RecoveredJournal)> {
        recover_rotation(path)?;
        reject_symlink(path)?;
        let recovered = if artifact_exists(path)? {
            replay(path, TornTail::Truncate)?
        } else {
            RecoveredJournal::default()
        };
        let file = open_append(path)?;
        let mut journal = Self {
            path: path.to_path_buf(),
            file: Some(file),
            state: recovered.clone(),
        };
        if journal.file_len()? >= COMPACT_AT_BYTES {
            // Recovery already has a complete authoritative state. A failed
            // maintenance attempt leaves that generation replayable.
            if let Err(error) = journal.compact() {
                if journal.file.is_none() {
                    return Err(error);
                }
            }
        }
        Ok((journal, recovered))
    }

    pub fn upsert_entry(&mut self, entry: &PersistedEntry) -> io::Result<()> {
        self.append(Action::UpsertEntry(entry.clone()))
    }

    pub fn forget_entry(&mut self, remote_path: &str) -> io::Result<()> {
        self.append(Action::ForgetEntry {
            remote_path: remote_path.to_string(),
        })
    }

    pub fn move_entry(&mut self, old_path: &str, entry: &PersistedEntry) -> io::Result<()> {
        self.append(Action::MoveEntry {
            old_path: old_path.to_string(),
            entry: entry.clone(),
        })
    }

    pub fn upsert_delete(&mut self, delete: &PersistedDelete) -> io::Result<()> {
        self.append(Action::UpsertDelete(delete.clone()))
    }

    pub fn forget_delete(&mut self, token: u64) -> io::Result<()> {
        self.append(Action::ForgetDelete { token })
    }

    pub fn upsert_namespace_conflict(&mut self, intent: &NamespaceIntent) -> io::Result<()> {
        self.append(Action::UpsertNamespaceConflict(intent.clone()))
    }

    pub fn forget_namespace_conflict(&mut self, path: &str) -> io::Result<()> {
        self.append(Action::ForgetNamespaceConflict { path: path.into() })
    }

    fn append(&mut self, action: Action) -> io::Result<()> {
        reject_symlink(&self.path)?;
        let bytes = encode_record(action.clone())?;
        if self.file_len()?.saturating_add(bytes.len() as u64) > MAX_JOURNAL_BYTES {
            self.compact()?;
        }
        if self.file_len()?.saturating_add(bytes.len() as u64) > MAX_JOURNAL_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "mount journal state exceeds its bounded size; dirty state was preserved",
            ));
        }
        let durability = {
            let file = self.file_mut()?;
            file.write_all(&bytes).and_then(|()| file.sync_data())
        };
        if let Err(error) = durability {
            // The final record may be torn or durable despite the error. Do
            // not append behind that uncertainty in this process. Reopening
            // replays only complete newline-terminated records and truncates a
            // torn tail before accepting another mutation.
            drop(self.file.take());
            return Err(error);
        }
        apply(&mut self.state, action);
        if self.file_len()? >= COMPACT_AT_BYTES {
            // Compaction is maintenance after the logical append is durable.
            // All failure points retain an old or new replayable generation.
            if let Err(error) = self.compact() {
                if self.file.is_none() {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn compact(&mut self) -> io::Result<()> {
        let (next, old) = rotation_paths(&self.path)?;
        reject_symlink(&self.path)?;
        reject_symlink(&next)?;
        reject_symlink(&old)?;
        if artifact_exists(&next)? || artifact_exists(&old)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "mount journal has unfinished rotation artifacts",
            ));
        }

        let mut snapshot = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&next)?;
        let snapshot_result = write_snapshot(&mut snapshot, &self.state);
        if let Err(error) = snapshot_result {
            drop(snapshot);
            let _ = remove_plain_file(&next);
            return Err(error);
        }
        if let Err(error) = snapshot.sync_all() {
            drop(snapshot);
            let _ = remove_plain_file(&next);
            return Err(error);
        }
        drop(snapshot);
        sync_parent(&self.path);

        drop(self.file.take());
        if let Err(error) = fs::rename(&self.path, &old) {
            let _ = remove_plain_file(&next);
            self.file = open_append(&self.path).ok();
            return Err(error);
        }
        sync_parent(&self.path);
        if let Err(error) = fs::rename(&next, &self.path) {
            let restored = fs::rename(&old, &self.path).is_ok();
            let _ = remove_plain_file(&next);
            if restored {
                self.file = open_append(&self.path).ok();
            }
            return Err(error);
        }
        sync_parent(&self.path);
        self.file = Some(open_append(&self.path)?);
        remove_plain_file(&old)?;
        sync_parent(&self.path);
        Ok(())
    }

    fn file_len(&self) -> io::Result<u64> {
        self.file
            .as_ref()
            .ok_or_else(rotation_incomplete)?
            .metadata()
            .map(|metadata| metadata.len())
    }

    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file.as_mut().ok_or_else(rotation_incomplete)
    }
}

fn write_snapshot(file: &mut File, state: &RecoveredJournal) -> io::Result<()> {
    let mut written = 0u64;
    for entry in state.entries.values() {
        write_snapshot_record(file, Action::UpsertEntry(entry.clone()), &mut written)?;
    }
    for delete in state.deletes.values() {
        write_snapshot_record(file, Action::UpsertDelete(delete.clone()), &mut written)?;
    }
    for conflict in state.namespace_conflicts.values() {
        write_snapshot_record(
            file,
            Action::UpsertNamespaceConflict(conflict.clone()),
            &mut written,
        )?;
    }
    Ok(())
}

fn write_snapshot_record(file: &mut File, action: Action, written: &mut u64) -> io::Result<()> {
    let bytes = encode_record(action)?;
    *written = written.saturating_add(bytes.len() as u64);
    if *written > MAX_JOURNAL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mount journal snapshot exceeds its bounded size",
        ));
    }
    file.write_all(&bytes)
}

fn encode_record(action: Action) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(&Record {
        version: JOURNAL_VERSION,
        action,
    })
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mount journal record exceeds its size limit",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

fn replay(path: &Path, torn_tail: TornTail) -> io::Result<RecoveredJournal> {
    if !artifact_exists(path)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "mount journal does not exist",
        ));
    }
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_JOURNAL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mount journal exceeds its size limit",
        ));
    }
    let file = match torn_tail {
        TornTail::Truncate => OpenOptions::new().read(true).write(true).open(path)?,
        TornTail::Reject => File::open(path)?,
    };
    let mut reader = BufReader::new(file);
    let mut state = RecoveredJournal::default();
    let mut line = Vec::new();
    let mut complete_len = 0u64;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        let terminated = line.ends_with(b"\n");
        let record_limit = if terminated {
            MAX_RECORD_BYTES + 1
        } else {
            MAX_RECORD_BYTES
        };
        if line.len() > record_limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mount journal record exceeds its size limit",
            ));
        }
        if !terminated {
            return match torn_tail {
                TornTail::Truncate => {
                    let file = reader.into_inner();
                    file.set_len(complete_len)?;
                    file.sync_all()?;
                    sync_parent(path);
                    Ok(state)
                }
                TornTail::Reject => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mount journal compact snapshot has a torn final record",
                )),
            };
        }
        line.pop();
        let record: Record = serde_json::from_slice(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if record.version != JOURNAL_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported mount journal version",
            ));
        }
        apply(&mut state, record.action);
        complete_len += read as u64;
    }
    Ok(state)
}

fn apply(state: &mut RecoveredJournal, action: Action) {
    match action {
        Action::UpsertEntry(entry) => {
            state.entries.insert(entry.remote_path.clone(), entry);
        }
        Action::MoveEntry { old_path, entry } => {
            state.entries.remove(&old_path);
            state.entries.insert(entry.remote_path.clone(), entry);
        }
        Action::ForgetEntry { remote_path } => {
            state.entries.remove(&remote_path);
        }
        Action::UpsertDelete(delete) => {
            state.deletes.insert(delete.token, delete);
        }
        Action::UpsertNamespaceConflict(intent) => {
            state
                .namespace_conflicts
                .insert(intent.conflict.path.clone(), intent);
        }
        Action::ForgetNamespaceConflict { path } => {
            state.namespace_conflicts.remove(&path);
        }
        Action::ForgetDelete { token } => {
            state.deletes.remove(&token);
        }
    }
}

fn recover_rotation(path: &Path) -> io::Result<()> {
    let (next, old) = rotation_paths(path)?;
    let primary_exists = artifact_exists(path)?;
    let next_exists = artifact_exists(&next)?;
    let old_exists = artifact_exists(&old)?;
    if primary_exists {
        replay(path, TornTail::Truncate)?;
        if next_exists {
            remove_plain_file(&next)?;
        }
        if old_exists {
            remove_plain_file(&old)?;
        }
        return Ok(());
    }
    if old_exists {
        replay(&old, TornTail::Truncate)?;
        fs::rename(&old, path)?;
        if next_exists {
            remove_plain_file(&next)?;
        }
        sync_parent(path);
        return Ok(());
    }
    if next_exists {
        replay(&next, TornTail::Reject)?;
        fs::rename(&next, path)?;
        sync_parent(path);
    }
    Ok(())
}

fn rotation_paths(path: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "mount journal has no UTF-8 file name",
            )
        })?;
    Ok((
        path.with_file_name(format!("{name}.compact-new")),
        path.with_file_name(format!("{name}.compact-old")),
    ))
}

fn artifact_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "mount journal artifact is not a plain file: {}",
                    path.display()
                ),
            ))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn remove_plain_file(path: &Path) -> io::Result<()> {
    if artifact_exists(path)? {
        fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    reject_symlink(path)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "mount journal may not be a symbolic link",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
}

fn rotation_incomplete() -> io::Error {
    io::Error::other("mount journal rotation is incomplete; restart will recover it")
}
