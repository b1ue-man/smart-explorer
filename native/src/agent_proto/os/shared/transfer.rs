use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::fs::{is_pseudo_dir, systemtime_ms};
use super::promotion::{
    ensure_destination_parent_plain, promote_staged_replace, validate_destination_root,
};
use super::put_tree::TreeManifestValidator;
use super::session::{emit, Sink};
use super::{Frame, ValidatedRelativePath, CHUNK};

/// Read a file `[offset, offset+len)` (len 0 = to EOF) -> `Data`* then `End`.
pub(crate) fn handle_read(
    sink: &Sink,
    id: u64,
    path: &str,
    offset: u64,
    len: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    if offset > 0 {
        f.seek(SeekFrom::Start(offset))?;
    }
    let mut remaining = if len == 0 { u64::MAX } else { len };
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let want = remaining.min(buf.len() as u64) as usize;
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
        remaining -= n as u64;
    }
    emit(sink, id, &Frame::End)
}

/// Receive a byte stream (`Data`* `End`) into `path` via a temp + atomic rename.
pub(crate) fn handle_write(
    sink: &Sink,
    id: u64,
    path: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (tmp, mut f) = create_staged_file(path, "write", id)?;
    if let Err(error) = emit(sink, id, &Frame::Progress { done: 0, total: 0 }) {
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    let transfer = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
        }
        match inbound.recv_timeout(Duration::from_millis(100)) {
            Ok(Frame::Data(d)) if d.len() <= CHUNK => {
                if let Err(error) = f.write_all(&d) {
                    break Err(error);
                }
            }
            Ok(Frame::Data(_)) => {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "upload data frame exceeds the protocol chunk limit",
                ));
            }
            Ok(Frame::End) => {
                if cancel.load(Ordering::Relaxed) {
                    break Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
                }
                break Ok(());
            }
            Ok(_) => {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected frame in upload stream",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "upload aborted",
                ));
            }
        }
    };
    let transfer = transfer.and_then(|()| f.sync_all());
    drop(f);
    if let Err(error) = transfer {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = promote_staged_replace(&tmp, Path::new(path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    emit(sink, id, &Frame::Ok)
}

pub(crate) fn copy_file_safe(source: &str, destination: &str, id: u64) -> io::Result<u64> {
    let (staged, mut writer) = create_staged_file(destination, "copy", id)?;
    let copied = (|| {
        let mut reader = std::fs::File::open(source)?;
        let copied = io::copy(&mut reader, &mut writer)?;
        writer.sync_all()?;
        Ok(copied)
    })();
    drop(writer);
    let result = copied.and_then(|copied| {
        promote_staged_replace(&staged, Path::new(destination))?;
        Ok(copied)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

pub(crate) fn create_staged_file(
    destination: &str,
    purpose: &str,
    id: u64,
) -> io::Result<(std::path::PathBuf, std::fs::File)> {
    ensure_destination_parent_plain(Path::new(destination))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..1000u32 {
        let path = std::path::PathBuf::from(format!(
            "{destination}.se-agent-{purpose}-{id:x}-{nonce:x}-{attempt:x}.part"
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                if let Err(error) = super::local_platform::secure_staging_file(&file) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error);
                }
                return Ok((path, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate agent staged file",
    ))
}

pub(crate) fn remove_path(path: &str, recursive: bool) -> io::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.is_dir() {
        if recursive {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_dir(path)
        }
    } else {
        std::fs::remove_file(path)
    }
}

#[derive(Clone)]
pub(crate) struct LocalTreeEntry {
    pub(crate) relative: ValidatedRelativePath,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
    pub(crate) mtime_ms: i64,
    identity: Option<super::local_platform::FileIdentity>,
    modified: Option<std::time::SystemTime>,
}

/// Build a bounded, link-free manifest before emitting the first tree frame.
pub(crate) fn collect_local_tree(
    root: &Path,
    cancel: &AtomicBool,
) -> io::Result<Vec<LocalTreeEntry>> {
    validate_destination_root(root)?;
    let root_metadata = std::fs::symlink_metadata(root)?;
    if super::local_platform::metadata_is_link_like(&root_metadata) || !root_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bulk source root must be a plain directory",
        ));
    }
    if root.to_str().is_some_and(is_pseudo_dir) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bulk transfer of a pseudo-filesystem is unsupported",
        ));
    }

    let mut manifest = TreeManifestValidator::default();
    let mut entries = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(parent_relative) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "bulk source collection canceled",
            ));
        }
        let directory = if parent_relative.is_empty() {
            root.to_path_buf()
        } else {
            ValidatedRelativePath::parse(&parent_relative)?.join_local(root)
        };
        let directory_metadata = std::fs::symlink_metadata(&directory)?;
        if super::local_platform::metadata_is_link_like(&directory_metadata)
            || !directory_metadata.is_dir()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bulk source directory changed type during preflight",
            ));
        }
        let mut child_names = HashSet::new();
        for entry in std::fs::read_dir(&directory)? {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "bulk source collection canceled",
                ));
            }
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bulk source name is not valid UTF-8",
                )
            })?;
            ValidatedRelativePath::parse(&name)?;
            if !child_names.insert(name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bulk source directory contains duplicate child names",
                ));
            }
            let relative_text = if parent_relative.is_empty() {
                name
            } else {
                format!("{parent_relative}/{name}")
            };
            let relative = ValidatedRelativePath::parse(&relative_text)?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if super::local_platform::metadata_is_link_like(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bulk source contains a link-like entry: {relative_text}"),
                ));
            }
            if metadata.is_dir() {
                if path.to_str().is_some_and(is_pseudo_dir) {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "bulk source contains a pseudo-filesystem",
                    ));
                }
                manifest.record(&relative, true)?;
                entries.push(LocalTreeEntry {
                    relative,
                    is_dir: true,
                    size: 0,
                    mtime_ms: 0,
                    identity: None,
                    modified: None,
                });
                stack.push(relative_text);
            } else if metadata.is_file() {
                let file = std::fs::File::open(&path)?;
                let opened_metadata = file.metadata()?;
                let identity = super::local_platform::file_identity(&file)?;
                if !super::local_platform::path_matches_identity(&path, identity)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "bulk source changed identity during preflight",
                    ));
                }
                manifest.record(&relative, false)?;
                entries.push(LocalTreeEntry {
                    relative,
                    is_dir: false,
                    size: opened_metadata.len(),
                    mtime_ms: opened_metadata
                        .modified()
                        .ok()
                        .map(systemtime_ms)
                        .unwrap_or(0),
                    identity: Some(identity),
                    modified: opened_metadata.modified().ok(),
                });
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bulk source contains a special entry: {relative_text}"),
                ));
            }
        }
        let after = std::fs::symlink_metadata(&directory)?;
        if super::local_platform::metadata_is_link_like(&after) || !after.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bulk source directory changed type during preflight",
            ));
        }
    }
    Ok(entries)
}

pub(crate) fn open_local_tree_file(
    root: &Path,
    entry: &LocalTreeEntry,
) -> io::Result<std::fs::File> {
    validate_local_source_ancestors(root, entry)?;
    let path = entry.relative.join_local(root);
    let expected = entry
        .identity
        .ok_or_else(|| io::Error::other("bulk directory cannot be opened as a file"))?;
    let link_metadata = std::fs::symlink_metadata(&path)?;
    if super::local_platform::metadata_is_link_like(&link_metadata) || !link_metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bulk source changed into a link or non-file",
        ));
    }
    let file = std::fs::File::open(&path)?;
    let metadata = file.metadata()?;
    if super::local_platform::file_identity(&file)? != expected
        || metadata.len() != entry.size
        || metadata.modified().ok() != entry.modified
        || !super::local_platform::path_matches_identity(&path, expected)?
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bulk source changed before it was read",
        ));
    }
    Ok(file)
}

pub(crate) fn finish_local_tree_file(
    root: &Path,
    entry: &LocalTreeEntry,
    file: &std::fs::File,
    sent: u64,
) -> io::Result<()> {
    validate_local_source_ancestors(root, entry)?;
    let path = entry.relative.join_local(root);
    let expected = entry
        .identity
        .ok_or_else(|| io::Error::other("bulk directory was read as a file"))?;
    let metadata = file.metadata()?;
    let link_metadata = std::fs::symlink_metadata(&path)?;
    if sent != entry.size
        || metadata.len() != entry.size
        || metadata.modified().ok() != entry.modified
        || super::local_platform::file_identity(file)? != expected
        || super::local_platform::metadata_is_link_like(&link_metadata)
        || !super::local_platform::path_matches_identity(&path, expected)?
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bulk source changed while it was read",
        ));
    }
    Ok(())
}

fn validate_local_source_ancestors(root: &Path, entry: &LocalTreeEntry) -> io::Result<()> {
    validate_destination_root(root)?;
    let mut current = root.to_path_buf();
    let root_metadata = std::fs::symlink_metadata(&current)?;
    if super::local_platform::metadata_is_link_like(&root_metadata) || !root_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bulk source root changed into a link or non-directory",
        ));
    }
    let components: Vec<&str> = entry.relative.as_str().split('/').collect();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current)?;
        if super::local_platform::metadata_is_link_like(&metadata) || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bulk source ancestor changed into a link or non-directory",
            ));
        }
    }
    Ok(())
}

/// Stream an entire subtree down after a bounded, link-free preflight.
pub(crate) fn handle_get_tree(
    sink: &Sink,
    id: u64,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let entries = collect_local_tree(Path::new(root), cancel)?;
    let mut buffer = vec![0u8; CHUNK];
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "agent get-tree canceled",
            ));
        }
        emit(
            sink,
            id,
            &Frame::TreeEntry {
                rel: entry.relative.as_str().to_string(),
                is_dir: entry.is_dir,
                size: entry.size,
                mtime_ms: entry.mtime_ms,
            },
        )?;
        if entry.is_dir {
            continue;
        }
        let mut file = open_local_tree_file(Path::new(root), &entry)?;
        let mut sent = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "agent get-tree canceled",
                ));
            }
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            emit(sink, id, &Frame::Data(buffer[..read].to_vec()))?;
            sent = sent
                .checked_add(read as u64)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file size overflow"))?;
        }
        finish_local_tree_file(Path::new(root), &entry, &file, sent)?;
    }
    emit(sink, id, &Frame::End)
}

#[cfg(test)]
mod tree_tests {
    use super::collect_local_tree;
    use std::sync::atomic::AtomicBool;

    #[cfg(unix)]
    #[test]
    fn local_tree_preflight_rejects_links_and_literal_backslashes() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "se-agent-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(r"literal\name"), b"x").unwrap();
        assert!(collect_local_tree(&root, &AtomicBool::new(false)).is_err());
        std::fs::remove_file(root.join(r"literal\name")).unwrap();
        symlink("missing", root.join("link")).unwrap();
        assert!(collect_local_tree(&root, &AtomicBool::new(false)).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
