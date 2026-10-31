//! Extension frames (`ext-v1`) the agent serves on its own filesystem:
//! listings that keep going past single entries, hash walks that report
//! links, special files and unreadable entries as omissions (never a reason
//! to fail the walk), stage finishing and the questions about what this side
//! offers, and the SHA-256 of one file next to its data (`FILE_DIGEST`).
//! The storing host's duplicate search, recycling, target limits and
//! change subscriptions belong to the app's backends; the agent answers them
//! with `UNSUPPORTED_EXTENSION`.
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use super::fs::systemtime_ms;
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
/// `HashOmitted`, then `End` (`walk_hashed.rs`, on the host's threads). A
/// failure at the root fails the walk. SHA-256 takes one regular file.
pub(crate) fn handle_walk_hashed2(
    sink: &Sink,
    id: u64,
    root: &str,
    algorithm: u8,
    min_bytes: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if algorithm == digest::SHA256 {
        return walk_file_sha256(sink, id, root, cancel);
    }
    let form = super::walk_hashed::Form::Single;
    super::walk_hashed::walk(sink, id, root, algorithm, min_bytes, form, cancel)
}

/// `WalkHashed3` (`walk-meta-v1`): the walk of `WalkHashed2` in `HashPart`s;
/// with `meta` every entry carries its listing attributes and links and
/// special files are entries. A folder walk with SHA-256 stays unsupported.
pub(crate) fn handle_walk_hashed3(
    sink: &Sink,
    id: u64,
    root: &str,
    (algorithm, min_bytes, meta): (u8, u64, bool),
    cancel: &AtomicBool,
) -> io::Result<()> {
    if algorithm == digest::SHA256 {
        return emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    let form = super::walk_hashed::Form::Parts { meta };
    super::walk_hashed::walk(sink, id, root, algorithm, min_bytes, form, cancel)
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

/// `WalkHashed2` with SHA-256 (`query::FILE_DIGEST`): a regular-file root is
/// answered as one `HashEntry` with an empty `rel`, then `End`. A folder
/// root stays unsupported as before; a link, special, missing or changing
/// file fails the request instead of yielding a digest.
fn walk_file_sha256(sink: &Sink, id: u64, root: &str, cancel: &AtomicBool) -> io::Result<()> {
    let path = Path::new(root);
    let metadata = std::fs::symlink_metadata(path)?;
    let (link, special) = super::local_platform::metadata_class(path, &metadata);
    if metadata.is_dir() && !link && !special {
        return emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    if link || special || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Wurzel ist keine reguläre Datei: {root}"),
        ));
    }
    let mut context = super::sha256::Sha256::new();
    read_regular_exact(path, metadata.len(), cancel, &mut |chunk| {
        context.update(chunk)
    })?;
    let mtime_ms = metadata.modified().ok().map(systemtime_ms).unwrap_or(0);
    let entry = entry_frame(
        String::new(),
        false,
        metadata.len(),
        mtime_ms,
        Some(context.finish_hex()),
    );
    emit(sink, id, &entry)?;
    emit(sink, id, &Frame::End)
}

/// MD5 of a regular file, checking for a cancel between chunks.
pub(super) fn md5_file(path: &Path, expected_size: u64, cancel: &AtomicBool) -> io::Result<String> {
    let mut context = super::hash::Md5::new();
    read_regular_exact(path, expected_size, cancel, &mut |chunk| {
        context.update(chunk)
    })?;
    Ok(context.finish_hex())
}

/// Every byte of the regular file `path` into `consume`; exactly
/// `expected_size` bytes with unchanged size and time, or an error.
fn read_regular_exact(
    path: &Path,
    expected_size: u64,
    cancel: &AtomicBool,
    consume: &mut dyn FnMut(&[u8]),
) -> io::Result<()> {
    let mut file = super::local_platform::open_regular_no_follow(path, false)?;
    let before = file.metadata()?;
    let mut bytes = 0u64;
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
        consume(&buffer[..read]);
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
    Ok(())
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

pub(super) fn set_mtime(file: &std::fs::File, ms: i64) -> io::Result<()> {
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
        query::HASH_WALK | query::REMOVE_TREE | query::FILE_DIGEST => 1,
        query::SYNC_FILESYSTEM => {
            u64::from(super::local_platform::sync_filesystem(Path::new(path))?)
        }
        _ => 0,
    };
    Ok(Frame::Answer(value))
}
