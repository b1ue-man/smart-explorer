//! The drive journal `journal.jsonl`: append, replay, compaction and
//! rotation recovery. The record types and the path-sealing hook (contract
//! V1) live in `journal_record.rs`; every record is sealed on its way to disk
//! and unsealed on replay, so the in-memory state is always plain.
use super::journal_record::{apply, decode_record, encode_record, Action, MAX_RECORD_BYTES};
use super::path_sealer::PathSealer;
use super::types::NamespaceIntent;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) use super::journal_record::{
    DeletePhase, PersistedDelete, PersistedEntry, RecoveredJournal,
};

const COMPACT_AT_BYTES: u64 = 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
enum TornTail {
    Truncate,
    Reject,
}

pub(super) struct Journal {
    path: PathBuf,
    file: Option<File>,
    state: RecoveredJournal,
    sealer: Arc<dyn PathSealer>,
}

impl Journal {
    pub fn open(path: &Path, sealer: Arc<dyn PathSealer>) -> io::Result<(Self, RecoveredJournal)> {
        recover_rotation(path, &*sealer)?;
        reject_symlink(path)?;
        let recovered = if artifact_exists(path)? {
            replay(path, TornTail::Truncate, &*sealer)?
        } else {
            RecoveredJournal::default()
        };
        let file = open_append(path)?;
        let mut journal = Self {
            path: path.to_path_buf(),
            file: Some(file),
            state: recovered.clone(),
            sealer,
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
        let bytes = encode_record(action.clone().sealed(&*self.sealer)?)?;
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
        let snapshot_result = write_snapshot(&mut snapshot, &self.state, &*self.sealer);
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

fn write_snapshot(
    file: &mut File,
    state: &RecoveredJournal,
    sealer: &dyn PathSealer,
) -> io::Result<()> {
    let mut written = 0u64;
    for entry in state.entries.values() {
        let action = Action::UpsertEntry(entry.clone()).sealed(sealer)?;
        write_snapshot_record(file, action, &mut written)?;
    }
    for delete in state.deletes.values() {
        let action = Action::UpsertDelete(delete.clone()).sealed(sealer)?;
        write_snapshot_record(file, action, &mut written)?;
    }
    for conflict in state.namespace_conflicts.values() {
        let action = Action::UpsertNamespaceConflict(conflict.clone()).sealed(sealer)?;
        write_snapshot_record(file, action, &mut written)?;
    }
    Ok(())
}

/// Writes one already sealed action of a compaction snapshot.
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

fn replay(
    path: &Path,
    torn_tail: TornTail,
    sealer: &dyn PathSealer,
) -> io::Result<RecoveredJournal> {
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
        let action = decode_record(&line)?.unsealed(sealer)?;
        apply(&mut state, action);
        complete_len += read as u64;
    }
    Ok(state)
}

fn recover_rotation(path: &Path, sealer: &dyn PathSealer) -> io::Result<()> {
    let (next, old) = rotation_paths(path)?;
    let primary_exists = artifact_exists(path)?;
    let next_exists = artifact_exists(&next)?;
    let old_exists = artifact_exists(&old)?;
    if primary_exists {
        replay(path, TornTail::Truncate, sealer)?;
        if next_exists {
            remove_plain_file(&next)?;
        }
        if old_exists {
            remove_plain_file(&old)?;
        }
        return Ok(());
    }
    if old_exists {
        replay(&old, TornTail::Truncate, sealer)?;
        fs::rename(&old, path)?;
        if next_exists {
            remove_plain_file(&next)?;
        }
        sync_parent(path);
        return Ok(());
    }
    if next_exists {
        replay(&next, TornTail::Reject, sealer)?;
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
