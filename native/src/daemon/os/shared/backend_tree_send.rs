use std::collections::HashSet;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::agent_proto::{Frame, TreeManifestValidator, ValidatedRelativePath, CHUNK};
use crate::vfs::{BackendHandle, VfsMeta};

use super::backend_server::{emit, Sink};
use super::backend_transfer::{canonical_backend_root, validate_backend_destination};

struct SourceEntry {
    relative: ValidatedRelativePath,
    metadata: VfsMeta,
}

pub(super) fn handle_get_tree_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let root = canonical_backend_root(backend, root);
    let root = root.as_ref();
    let entries = collect_source(backend, root, cancel)?;
    let mut buffer = vec![0u8; CHUNK];
    for entry in entries {
        check_canceled(cancel)?;
        let path = join_path(root, entry.relative.as_str());
        validate_source_ancestors(backend, root, &entry.relative)?;
        let fresh = backend.stat(&path)?;
        validate_same_entry(&entry.metadata, &fresh, &path)?;
        emit(
            sink,
            id,
            &Frame::TreeEntry {
                rel: entry.relative.as_str().to_string(),
                is_dir: fresh.is_dir,
                size: fresh.size,
                mtime_ms: fresh.mtime_ms,
            },
        )?;
        if fresh.is_dir {
            continue;
        }
        let mut reader = backend.open_read_id(&path, fresh.id.as_deref())?;
        let mut sent = 0u64;
        loop {
            check_canceled(cancel)?;
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            emit(sink, id, &Frame::Data(buffer[..read].to_vec()))?;
            sent = sent
                .checked_add(read as u64)
                .ok_or_else(|| invalid("backend tree file size overflow"))?;
        }
        if sent != fresh.size {
            return Err(invalid("backend tree source changed size while reading"));
        }
        validate_source_ancestors(backend, root, &entry.relative)?;
        let after = backend.stat(&path)?;
        validate_same_entry(&fresh, &after, &path)?;
    }
    emit(sink, id, &Frame::End)
}

fn collect_source(
    backend: &BackendHandle,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<Vec<SourceEntry>> {
    validate_backend_destination(backend, root)?;
    let root_metadata = backend.stat(root)?;
    if root_metadata.is_symlink || !root_metadata.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "backend tree source root must be a plain directory",
        ));
    }
    let mut validator = TreeManifestValidator::default();
    let mut entries = Vec::new();
    let mut stack = vec![(String::new(), root_metadata)];
    while let Some((parent_relative, expected_directory)) = stack.pop() {
        check_canceled(cancel)?;
        let directory = if parent_relative.is_empty() {
            root.to_string()
        } else {
            join_path(root, &parent_relative)
        };
        validate_same_entry(&expected_directory, &backend.stat(&directory)?, &directory)?;
        let mut child_names = HashSet::new();
        for listed in backend.list_dir(&directory)? {
            check_canceled(cancel)?;
            crate::vfs::validate_child_name(&listed.name)?;
            ValidatedRelativePath::parse(&listed.name)?;
            if !child_names.insert(listed.name.clone()) {
                return Err(invalid("backend returned a duplicate tree child name"));
            }
            let path = join_path(&directory, &listed.name);
            let relative_text = if parent_relative.is_empty() {
                listed.name.clone()
            } else {
                format!("{parent_relative}/{}", listed.name)
            };
            let relative = ValidatedRelativePath::parse(&relative_text)?;
            let fresh = backend.stat(&path)?;
            if listed.is_symlink || fresh.is_symlink {
                return Err(invalid("backend tree source contains a link-like entry"));
            }
            if let (Some(listed_id), Some(fresh_id)) = (listed.id.as_deref(), fresh.id.as_deref()) {
                if listed_id != fresh_id {
                    return Err(invalid(
                        "backend tree entry identity changed during preflight",
                    ));
                }
            }
            validator.record(&relative, fresh.is_dir)?;
            entries.push(SourceEntry {
                relative,
                metadata: fresh.clone(),
            });
            if fresh.is_dir {
                stack.push((relative_text, fresh));
            }
        }
        validate_same_entry(&expected_directory, &backend.stat(&directory)?, &directory)?;
    }
    Ok(entries)
}

fn validate_source_ancestors(
    backend: &BackendHandle,
    root: &str,
    relative: &ValidatedRelativePath,
) -> io::Result<()> {
    validate_backend_destination(backend, root)?;
    let root_metadata = backend.stat(root)?;
    if root_metadata.is_symlink || !root_metadata.is_dir {
        return Err(invalid(
            "backend tree source root changed into a link-like entry",
        ));
    }
    let mut current = root.to_string();
    let components: Vec<&str> = relative.as_str().split('/').collect();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current = join_path(&current, component);
        let metadata = backend.stat(&current)?;
        if metadata.is_symlink || !metadata.is_dir {
            return Err(invalid("backend tree source ancestor became link-like"));
        }
    }
    Ok(())
}

fn validate_same_entry(expected: &VfsMeta, actual: &VfsMeta, path: &str) -> io::Result<()> {
    let identity_changed = match expected.id.as_deref() {
        Some(expected) => actual.id.as_deref() != Some(expected),
        None => false,
    };
    let time_changed =
        expected.mtime_ms != 0 && actual.mtime_ms != 0 && expected.mtime_ms != actual.mtime_ms;
    if expected.is_dir != actual.is_dir
        || expected.is_symlink != actual.is_symlink
        || actual.is_symlink
        || (!expected.is_dir && expected.size != actual.size)
        || identity_changed
        || time_changed
    {
        return Err(invalid(&format!(
            "backend tree source changed during transfer: {path}"
        )));
    }
    Ok(())
}

fn join_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}

fn check_canceled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "daemon backend get-tree canceled",
        ))
    } else {
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
