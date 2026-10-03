//! Extension frames (`ext-v1`) the agent serves on its own filesystem:
//! listings that keep going past single entries, hash walks that report
//! links, special files and unreadable entries as omissions (never a reason
//! to fail the walk), stage finishing and the questions about what this side
//! offers. The storing host's duplicate search, recycling, target limits and
//! change subscriptions belong to the app's backends; the agent answers them
//! with `UNSUPPORTED_EXTENSION`.
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use super::fs::{is_pseudo_dir, systemtime_ms};
use super::ops_types::{digest, omission, query};
use super::session::{emit, Sink};
use super::{Frame, WireMeta, WireOmission, CHUNK};

/// Encoded bytes after which a listing part is sent.
const PART_BYTES: usize = 1024 * 1024;

fn canceled(operation: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, format!("{operation} canceled"))
}

/// The omission code of a failure while reading one entry.
fn failure_reason(error: &io::Error) -> u8 {
    if error.kind() == io::ErrorKind::NotFound {
        omission::VANISHED
    } else {
        omission::UNREADABLE
    }
}

fn omitted(rel: String, reason: u8, detail: impl Into<String>) -> WireOmission {
    WireOmission {
        rel,
        reason,
        detail: detail.into(),
    }
}

/// `ListTolerant`: `DirPart`* then `End`. Only a failure of the enumeration
/// itself fails the listing.
pub(crate) fn handle_list_tolerant(
    sink: &Sink,
    id: u64,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut entries = Vec::new();
    let mut omissions = Vec::new();
    let mut part_bytes = 0usize;
    for entry in std::fs::read_dir(path)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled("agent listing"));
        }
        let entry = entry?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(raw) => {
                let rel = raw.to_string_lossy().into_owned();
                part_bytes += rel.len() + 32;
                omissions.push(omitted(
                    rel,
                    omission::UNREPRESENTABLE,
                    "Name ist kein gültiges UTF-8",
                ));
                if part_bytes >= PART_BYTES {
                    send_part(sink, id, &mut entries, &mut omissions)?;
                    part_bytes = 0;
                }
                continue;
            }
        };
        let path = entry.path();
        part_bytes += name.len() + 32;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => entries.push(meta(name, &path, &metadata)),
            Err(error) => omissions.push(omitted(name, failure_reason(&error), error.to_string())),
        }
        if part_bytes >= PART_BYTES {
            send_part(sink, id, &mut entries, &mut omissions)?;
            part_bytes = 0;
        }
    }
    if !entries.is_empty() || !omissions.is_empty() {
        send_part(sink, id, &mut entries, &mut omissions)?;
    }
    emit(sink, id, &Frame::End)
}

fn send_part(
    sink: &Sink,
    id: u64,
    entries: &mut Vec<WireMeta>,
    omissions: &mut Vec<WireOmission>,
) -> io::Result<()> {
    emit(
        sink,
        id,
        &Frame::DirPart {
            entries: std::mem::take(entries),
            omitted: std::mem::take(omissions),
        },
    )
}

fn meta(name: String, path: &Path, metadata: &std::fs::Metadata) -> WireMeta {
    let (link, special) = super::local_platform::metadata_class(path, metadata);
    WireMeta {
        name,
        is_dir: metadata.is_dir() && !link && !special,
        is_symlink: link,
        special,
        size: if special { 0 } else { metadata.len() },
        mtime_ms: metadata.modified().ok().map(systemtime_ms).unwrap_or(0),
        content_md5: None,
    }
}

/// `WalkHashed2`: every folder and regular file of at least `min_bytes`
/// (with its MD5 when asked), links, special files and unreadable entries as
/// `HashOmitted`, then `End`. A failure at the root fails the walk.
pub(crate) fn handle_walk_hashed2(
    sink: &Sink,
    id: u64,
    root: &str,
    algorithm: u8,
    min_bytes: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if algorithm == digest::SHA256 {
        return emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    if !matches!(algorithm, digest::NONE | digest::MD5) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unbekannter Hashalgorithmus",
        ));
    }
    let base = Path::new(root);
    let metadata = std::fs::symlink_metadata(base)?;
    if super::local_platform::metadata_is_link_like(base, &metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Wurzel ist kein Ordner: {root}"),
        ));
    }
    let mut stack: Vec<(PathBuf, String)> = vec![(base.to_path_buf(), String::new())];
    while let Some((dir, rel_dir)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled("agent hash walk"));
        }
        // Re-check queued children: a link inserted after the parent's
        // listing is an omission, never a directory traversal outside it.
        let current = match std::fs::symlink_metadata(&dir) {
            Ok(current) => current,
            Err(error) if rel_dir.is_empty() => return Err(error),
            Err(error) => {
                emit_omission(sink, id, rel_dir, failure_reason(&error), error.to_string())?;
                continue;
            }
        };
        if super::local_platform::metadata_is_link_like(&dir, &current) {
            emit_omission(sink, id, rel_dir, omission::LINK, "Verknüpfung")?;
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if rel_dir.is_empty() => return Err(error),
            Err(error) => {
                emit_omission(sink, id, rel_dir, failure_reason(&error), error.to_string())?;
                continue;
            }
        };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled("agent hash walk"));
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    // The enumeration of this folder broke off: the rest of
                    // it is unknown, never absent.
                    emit_omission(
                        sink,
                        id,
                        rel_dir.clone(),
                        omission::UNREADABLE,
                        error.to_string(),
                    )?;
                    break;
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel_dir.is_empty() {
                name.clone()
            } else {
                format!("{rel_dir}/{name}")
            };
            if entry.file_name().to_str().is_none() {
                emit_omission(
                    sink,
                    id,
                    rel,
                    omission::UNREPRESENTABLE,
                    "Name ist kein gültiges UTF-8",
                )?;
                continue;
            }
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    emit_omission(sink, id, rel, failure_reason(&error), error.to_string())?;
                    continue;
                }
            };
            let (link, special) = super::local_platform::metadata_class(&path, &metadata);
            if link {
                emit_omission(sink, id, rel, omission::LINK, "Verknüpfung")?;
                continue;
            }
            let mtime_ms = metadata.modified().ok().map(systemtime_ms).unwrap_or(0);
            if metadata.is_dir() {
                if path.to_str().is_some_and(is_pseudo_dir) {
                    continue;
                }
                emit(sink, id, &entry_frame(rel.clone(), true, 0, mtime_ms, None))?;
                stack.push((path, rel));
                continue;
            }
            if special || !metadata.is_file() {
                emit_omission(sink, id, rel, omission::SPECIAL, "Pipe, Socket oder Gerät")?;
                continue;
            }
            if metadata.len() < min_bytes {
                continue;
            }
            let md5 = if algorithm == digest::MD5 {
                match md5_file(&path, metadata.len(), cancel) {
                    Ok(md5) => Some(md5),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                    Err(error) => {
                        emit_omission(sink, id, rel, failure_reason(&error), error.to_string())?;
                        continue;
                    }
                }
            } else {
                None
            };
            emit(
                sink,
                id,
                &entry_frame(rel, false, metadata.len(), mtime_ms, md5),
            )?;
        }
    }
    emit(sink, id, &Frame::End)
}

fn entry_frame(rel: String, is_dir: bool, size: u64, mtime_ms: i64, md5: Option<String>) -> Frame {
    Frame::HashEntry {
        rel,
        is_dir,
        size,
        mtime_ms,
        md5,
    }
}

fn emit_omission(
    sink: &Sink,
    id: u64,
    rel: String,
    reason: u8,
    detail: impl Into<String>,
) -> io::Result<()> {
    emit(sink, id, &Frame::HashOmitted(omitted(rel, reason, detail)))
}

/// MD5 of a regular file, checking for a cancel between chunks.
pub(super) fn md5_file(path: &Path, expected_size: u64, cancel: &AtomicBool) -> io::Result<String> {
    let mut file = super::local_platform::open_regular_no_follow(path, false)?;
    let before = file.metadata()?;
    let mut bytes = 0u64;
    let mut context = super::hash::Md5::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled("agent hash"));
        }
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        bytes = bytes.saturating_add(read as u64);
        context.update(&buffer[..read]);
    }
    let after = file.metadata()?;
    if bytes != expected_size
        || before.len() != expected_size
        || after.len() != expected_size
        || before.modified().ok() != after.modified().ok()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Datei während des Hashens geändert",
        ));
    }
    Ok(context.finish_hex())
}

/// `FinishStage`: time and mode of a complete stage (no error when the
/// filesystem keeps neither), then the requested flush (an error when it
/// fails). Returns `StageDone`.
pub(crate) fn finish_stage(
    stage: &str,
    mtime_ms: Option<i64>,
    mode: Option<u32>,
    durability: u8,
) -> io::Result<Frame> {
    let path = Path::new(stage);
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || super::local_platform::metadata_is_link_like(path, &metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Zwischendatei ist keine reguläre Datei: {stage}"),
        ));
    }
    let file = super::local_platform::open_regular_no_follow(path, true)?;
    let mtime_applied = match mtime_ms {
        Some(ms) => set_mtime(&file, ms).is_ok(),
        None => false,
    };
    if let Some(mode) = mode {
        // A filesystem without Unix modes is no error of the run.
        let _ = super::local_platform::set_file_mode(&file, mode);
    }
    let durable = durability != 0;
    if durable {
        file.sync_all()?;
    }
    Ok(Frame::StageDone {
        mtime_applied,
        durable,
    })
}

fn set_mtime(file: &std::fs::File, ms: i64) -> io::Result<()> {
    let magnitude = Duration::from_millis(ms.unsigned_abs());
    let time = if ms >= 0 {
        UNIX_EPOCH.checked_add(magnitude)
    } else {
        UNIX_EPOCH.checked_sub(magnitude)
    }
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "time out of range"))?;
    file.set_modified(time)
}

/// `Query`: what this agent offers below `path` (`Answer`).
pub(crate) fn answer(kind: u8, path: &str) -> io::Result<Frame> {
    let value = match kind {
        query::HASH_WALK => 1,
        query::SYNC_FILESYSTEM => {
            u64::from(super::local_platform::sync_filesystem(Path::new(path))?)
        }
        _ => 0,
    };
    Ok(Frame::Answer(value))
}
