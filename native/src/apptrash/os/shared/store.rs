//! Filesystem side of the app trash. Layout per volume:
//! `<volume>/.SmartExplorer-Papierkorb/<id>.json` (record) and
//! `<volume>/.SmartExplorer-Papierkorb/<id>/<name>` (payload).
//!
//! Order keeps every crash state safe: the record is claimed first (exclusive
//! create), then the slot directory, then the item is renamed in; deletion
//! removes the payload before the record. A record without payload is not
//! listed and is removed by the age purge. Moves use the no-replace rename of
//! the local backend, so neither trashing nor restoring ever overwrites.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::record::{
    entry_id, numbered_name, valid_id, volume_for, Record, TrashEntry, MAX_RECORD_BYTES,
};
use super::TRASH_DIR_NAME;

const DAY_MS: i64 = 86_400_000;
const MAX_ID_ATTEMPTS: u32 = 16;
const MAX_NAME_ATTEMPTS: u32 = 10_000;

pub(super) fn move_to_trash_in(volumes: &[PathBuf], path: &Path) -> io::Result<TrashEntry> {
    if !path.is_absolute() {
        return Err(invalid_input("Der Papierkorb braucht einen absoluten Pfad"));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid_input("Der Eintrag hat keinen gültigen Namen"))?
        .to_string();
    let parent = path
        .parent()
        .ok_or_else(|| invalid_input("Der Eintrag hat keinen übergeordneten Ordner"))?;
    let original = std::fs::canonicalize(parent)?.join(&name);
    let metadata = std::fs::symlink_metadata(&original)?;
    let volumes = canonical_volumes(volumes);
    let volume = volume_for(&volumes, &original).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Dieser Ort hat keinen Papierkorb",
        )
    })?;
    let root = volume.join(TRASH_DIR_NAME);
    if original.starts_with(&root) {
        return Err(invalid_input(
            "Einträge im Papierkorb werden dort endgültig gelöscht",
        ));
    }
    std::fs::create_dir_all(&root)?;
    let mut entry = TrashEntry {
        id: String::new(),
        name: name.clone(),
        original: original.clone(),
        deleted_ms: now_ms(),
        size: if metadata.is_dir() {
            tree_size(&original)
        } else {
            metadata.len()
        },
        is_dir: metadata.is_dir(),
    };
    claim_record(&root, &mut entry)?;
    let slot = root.join(&entry.id);
    let moved = std::fs::create_dir(&slot)
        .and_then(|()| crate::vfs::promote_local_copy(&original, &slot.join(&name)));
    if let Err(error) = moved {
        // Roll back the bookkeeping; the item itself was not moved.
        let _ = std::fs::remove_dir(&slot);
        let _ = std::fs::remove_file(record_path(&root, &entry.id));
        return Err(if error.kind() == io::ErrorKind::CrossesDevices {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Dieser Ort liegt auf einem anderen Dateisystem als sein Papierkorb",
            )
        } else {
            error
        });
    }
    Ok(entry)
}

/// Every intact entry, newest first. Damaged or planted records are not listed.
pub(super) fn list_in(volumes: &[PathBuf]) -> io::Result<Vec<TrashEntry>> {
    let mut entries = Vec::new();
    for volume in canonical_volumes(volumes) {
        let root = volume.join(TRASH_DIR_NAME);
        let directory = match std::fs::read_dir(&root) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for item in directory {
            let Some(id) = record_id(&item?.file_name()) else {
                continue;
            };
            let Some(record) = read_trusted(&root, &id, &volume) else {
                continue;
            };
            if std::fs::symlink_metadata(root.join(&id).join(&record.name)).is_ok() {
                entries.push(record.into_entry());
            }
        }
    }
    entries.sort_by(|left, right| {
        right
            .deleted_ms
            .cmp(&left.deleted_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(entries)
}

/// Moves the item back; an occupied original name becomes `Name (2)` etc.
pub(super) fn restore_in(volumes: &[PathBuf], id: &str) -> io::Result<PathBuf> {
    let (root, record) = locate(volumes, id)?;
    let payload = root.join(id).join(&record.name);
    std::fs::symlink_metadata(&payload)?;
    let parent = record
        .original
        .parent()
        .ok_or_else(|| invalid_input("Der ursprüngliche Ort ist ungültig"))?;
    std::fs::create_dir_all(parent)?;
    let mut index = 1;
    let target = loop {
        let candidate = parent.join(numbered_name(&record.name, record.is_dir, index));
        match crate::vfs::promote_local_copy(&payload, &candidate) {
            Ok(()) => break candidate,
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists && index < MAX_NAME_ATTEMPTS =>
            {
                index += 1;
            }
            Err(error) => return Err(error),
        }
    };
    // The item is back. The empty slot and its record are bookkeeping only; a
    // leftover record has no payload, is not listed, and the age purge drops it.
    let _ = std::fs::remove_dir(root.join(id));
    let _ = std::fs::remove_file(record_path(&root, id));
    Ok(target)
}

pub(super) fn delete_in(volumes: &[PathBuf], id: &str) -> io::Result<()> {
    let (root, _) = locate(volumes, id)?;
    remove_entry_files(&root, id)
}

/// Removes entries deleted at least `days` days before `now_ms`. Every due
/// entry is attempted; the first failure is reported after the pass.
pub(super) fn purge_older_than_in(
    volumes: &[PathBuf],
    days: u32,
    now_ms: i64,
) -> io::Result<usize> {
    let cutoff = now_ms.saturating_sub(i64::from(days).saturating_mul(DAY_MS));
    let mut removed = 0;
    let mut first_error = None;
    for volume in canonical_volumes(volumes) {
        let root = volume.join(TRASH_DIR_NAME);
        let directory = match std::fs::read_dir(&root) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                keep_first(&mut first_error, error);
                continue;
            }
        };
        for item in directory {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    keep_first(&mut first_error, error);
                    continue;
                }
            };
            let Some(id) = record_id(&item.file_name()) else {
                continue;
            };
            // Damaged records (e.g. an interrupted write) age by their file time.
            let deleted_ms = match read_trusted(&root, &id, &volume) {
                Some(record) => Some(record.deleted_ms),
                None => item
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .map(system_time_ms),
            };
            if deleted_ms.is_some_and(|deleted| deleted <= cutoff) {
                match remove_entry_files(&root, &id) {
                    Ok(()) => removed += 1,
                    Err(error) => {
                        keep_first(&mut first_error, error);
                    }
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(removed),
    }
}

fn keep_first(slot: &mut Option<io::Error>, error: io::Error) {
    if slot.is_none() {
        *slot = Some(error);
    }
}

pub(super) fn now_ms() -> i64 {
    system_time_ms(SystemTime::now())
}

fn system_time_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Claims a fresh id for `entry` by creating its record exclusively, then
/// writes the record.
fn claim_record(root: &Path, entry: &mut TrashEntry) -> io::Result<()> {
    for _ in 0..MAX_ID_ATTEMPTS {
        let id = entry_id(entry.deleted_ms, random_u64()?);
        let path = record_path(root, &id);
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        entry.id = id;
        let written = serde_json::to_vec(&Record::from_entry(entry))
            .map_err(io::Error::other)
            .and_then(|bytes| file.write_all(&bytes))
            .and_then(|()| file.sync_all());
        if let Err(error) = written {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Kein freier Papierkorb-Eintrag",
    ))
}

fn random_u64() -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| io::Error::other(format!("Zufallszahl: {error}")))?;
    Ok(u64::from_le_bytes(bytes))
}

/// Finds the volume trash holding `id` and its trusted record.
fn locate(volumes: &[PathBuf], id: &str) -> io::Result<(PathBuf, Record)> {
    if !valid_id(id) {
        return Err(invalid_input("Ungültige Papierkorb-Kennung"));
    }
    for volume in canonical_volumes(volumes) {
        let root = volume.join(TRASH_DIR_NAME);
        if std::fs::symlink_metadata(record_path(&root, id)).is_err() {
            continue;
        }
        return match read_trusted(&root, id, &volume) {
            Some(record) => Ok((root, record)),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Der Papierkorb-Eintrag ist beschädigt",
            )),
        };
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Eintrag nicht im Papierkorb",
    ))
}

fn read_trusted(root: &Path, id: &str, volume: &Path) -> Option<Record> {
    let file = std::fs::File::open(record_path(root, id)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return None;
    }
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    record.is_trusted(id, volume, root).then_some(record)
}

/// Payload first, record last, so an interrupted removal stays retryable.
/// `remove_dir_all` removes links inside the slot without following them.
fn remove_entry_files(root: &Path, id: &str) -> io::Result<()> {
    match std::fs::remove_dir_all(root.join(id)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match std::fs::remove_file(record_path(root, id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn record_id(file_name: &std::ffi::OsStr) -> Option<String> {
    let id = file_name.to_str()?.strip_suffix(".json")?;
    valid_id(id).then(|| id.to_string())
}

fn record_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.json"))
}

/// Volumes that exist right now (a removed SD card simply drops out).
fn canonical_volumes(volumes: &[PathBuf]) -> Vec<PathBuf> {
    volumes
        .iter()
        .filter_map(|volume| std::fs::canonicalize(volume).ok())
        .collect()
}

/// Bytes of all regular files below `root`, without following links. Best
/// effort: unreadable folders count as empty, the size is informational.
fn tree_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(items) = std::fs::read_dir(&directory) else {
            continue;
        };
        for item in items.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(item.path()) else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(item.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
