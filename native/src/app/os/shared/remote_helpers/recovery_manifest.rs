use super::temp::{
    read_session_pid, session_tag, session_temp_dir, temp_root, RemoteEdit, PRESERVE_MARKER,
};
use crate::app::app_models::TEMP_SESSION_PID_FILE;
use crate::app::platform_helpers::{process_running, replace_file_atomic};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::Path;

const RECOVERY_MANIFEST_SCHEMA: u32 = 2;
const MAX_RECOVERY_MANIFEST_BYTES: u64 = 1024 * 1024;
const LEGACY_MANIFEST_HEADER: &str =
    "Smart Explorer preserved this session because work may be unsaved.";

static MANIFEST_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StaleTempDisposition {
    Ignore,
    Cleanup,
    Recovery,
}

#[derive(Deserialize, Serialize)]
struct RecoveryManifest {
    schema: u32,
    entries: Vec<RecoveryManifestEntry>,
}

#[derive(Deserialize, Serialize)]
struct RecoveryManifestEntry {
    name: String,
    local: String,
    remote: String,
    dirty: bool,
    uploading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreserveMarkerState {
    Missing,
    KnownEmpty,
    Recovery,
    Unsafe,
}

pub(super) fn stale_temp_disposition(directory: &Path) -> StaleTempDisposition {
    // Startup sweeping and the Recovery UI deliberately share this one
    // owner/marker decision so a live session can never be counted by one and
    // removed by the other.
    let root = temp_root();
    if directory.parent() != Some(root.as_path())
        || directory.file_name().and_then(|name| name.to_str()) == Some(session_tag())
    {
        return StaleTempDisposition::Ignore;
    }
    let Ok(metadata) = std::fs::symlink_metadata(directory) else {
        return StaleTempDisposition::Ignore;
    };
    if !metadata.is_dir() || crate::app::upload_is_link_like(&metadata) {
        return StaleTempDisposition::Ignore;
    }
    match read_session_pid(directory) {
        Some(pid) if process_running(pid) => return StaleTempDisposition::Ignore,
        Some(_) => {}
        None => return StaleTempDisposition::Ignore,
    }
    match preserve_marker_state(directory) {
        PreserveMarkerState::Missing | PreserveMarkerState::KnownEmpty => {
            StaleTempDisposition::Cleanup
        }
        PreserveMarkerState::Recovery => StaleTempDisposition::Recovery,
        PreserveMarkerState::Unsafe => StaleTempDisposition::Ignore,
    }
}

fn preserve_marker_state(directory: &Path) -> PreserveMarkerState {
    let path = directory.join(PRESERVE_MARKER);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return PreserveMarkerState::Missing;
        }
        Err(_) => return PreserveMarkerState::Unsafe,
    };
    if !metadata.is_file() || crate::app::upload_is_link_like(&metadata) {
        return PreserveMarkerState::Unsafe;
    }
    if metadata.len() > MAX_RECOVERY_MANIFEST_BYTES {
        return PreserveMarkerState::Recovery;
    }
    let Ok(contents) = std::fs::read_to_string(path) else {
        return PreserveMarkerState::Recovery;
    };
    match recognized_manifest_entries(&contents) {
        Some(0) if !session_has_payload(directory) => PreserveMarkerState::KnownEmpty,
        // One declared edit proves that a real file was published previously.
        // Keep recovery even if an atomic editor save happens to be between its
        // delete and rename phases while startup inspects the directory.
        Some(_) | None => PreserveMarkerState::Recovery,
    }
}

fn recognized_manifest_entries(contents: &str) -> Option<usize> {
    if let Ok(manifest) = serde_json::from_str::<RecoveryManifest>(contents) {
        return (manifest.schema == RECOVERY_MANIFEST_SCHEMA).then_some(manifest.entries.len());
    }
    legacy_manifest_entries(contents)
}

fn legacy_manifest_entries(contents: &str) -> Option<usize> {
    if !contents.ends_with('\n') {
        return None;
    }
    let mut lines = contents.lines();
    if lines.next() != Some(LEGACY_MANIFEST_HEADER)
        || !matches!(
            lines.next(),
            Some("active_transfer=0" | "active_transfer=1")
        )
    {
        return None;
    }
    let mut entries = 0usize;
    loop {
        let Some(separator) = lines.next() else {
            return Some(entries);
        };
        if !separator.is_empty()
            || !lines.next().is_some_and(|line| line.starts_with("name="))
            || !lines.next().is_some_and(|line| line.starts_with("local="))
            || !lines.next().is_some_and(|line| line.starts_with("remote="))
            || !matches!(lines.next(), Some("dirty=0" | "dirty=1"))
            || !matches!(lines.next(), Some("uploading=0" | "uploading=1"))
        {
            return None;
        }
        entries = entries.saturating_add(1);
    }
}

fn session_has_payload(directory: &Path) -> bool {
    // An allocated editor directory is not recovery data until it contains a
    // regular file. Links, special files and unreadable trees remain fail-closed.
    let Ok(entries) = std::fs::read_dir(directory) else {
        return true;
    };
    let mut pending = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return true;
        };
        let name = entry.file_name();
        if name
            .to_str()
            .is_some_and(|name| name == TEMP_SESSION_PID_FILE || name == PRESERVE_MARKER)
        {
            continue;
        }
        pending.push(entry.path());
    }
    let mut inspected = 0usize;
    while let Some(path) = pending.pop() {
        inspected = inspected.saturating_add(1);
        if inspected > 100_000 {
            return true;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            return true;
        };
        if crate::app::upload_is_link_like(&metadata) {
            return true;
        }
        if metadata.is_file() {
            return true;
        }
        if !metadata.is_dir() {
            return true;
        }
        let Ok(children) = std::fs::read_dir(path) else {
            return true;
        };
        for child in children {
            let Ok(child) = child else {
                return true;
            };
            pending.push(child.path());
        }
    }
    false
}

pub(in crate::app) fn sync_recovery_manifest(remote_edits: &[RemoteEdit]) -> io::Result<()> {
    let directory = session_temp_dir();
    let mut entries = Vec::new();
    for edit in remote_edits {
        // ShellExecute can return a short-lived launcher even while a
        // single-instance editor (notably Obsidian/Electron) keeps the file
        // open and may save it later. Therefore process exit is never proof
        // that this managed temp copy is disposable. Only removing the
        // RemoteEdit from application state may remove its manifest entry.
        if let Some(entry) = recovery_manifest_entry(edit)? {
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return remove_current_manifest(&directory);
    }
    let manifest = RecoveryManifest {
        schema: RECOVERY_MANIFEST_SCHEMA,
        entries,
    };
    let contents = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if contents.len() as u64 > MAX_RECOVERY_MANIFEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery manifest exceeds its 1 MiB limit",
        ));
    }
    atomic_write_manifest(&directory, &contents)
}

fn recovery_manifest_entry(edit: &RemoteEdit) -> io::Result<Option<RecoveryManifestEntry>> {
    let directory = session_temp_dir();
    require_safe_session_directory(&directory)?;
    let Some(parent) = edit.temp.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "editor temp copy has no parent directory",
        ));
    };
    if parent.parent() != Some(directory.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "editor temp copy escaped its session: {}",
                edit.temp.display()
            ),
        ));
    }
    let parent_metadata = match std::fs::symlink_metadata(parent) {
        Ok(metadata) => metadata,
        // A registered edit had a real downloaded file before it was handed
        // to the editor. NotFound can be the brief delete/rename window of an
        // atomic Obsidian save, so fail closed and keep the previous manifest.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "editor temp parent is temporarily absent: {}",
                    parent.display()
                ),
            ));
        }
        Err(error) => return Err(error),
    };
    if !parent_metadata.is_dir() || crate::app::upload_is_link_like(&parent_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "editor temp parent is not a safe directory: {}",
                parent.display()
            ),
        ));
    }
    let metadata = match std::fs::symlink_metadata(&edit.temp) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "editor temp copy is temporarily absent: {}",
                    edit.temp.display()
                ),
            ));
        }
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || crate::app::upload_is_link_like(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "editor temp copy is not a safe regular file: {}",
                edit.temp.display()
            ),
        ));
    }
    Ok(Some(RecoveryManifestEntry {
        name: edit.name.clone(),
        local: edit.temp.to_string_lossy().into_owned(),
        remote: edit.remote_path.clone(),
        dirty: edit.dirty,
        uploading: edit.uploading,
    }))
}

fn remove_current_manifest(directory: &Path) -> io::Result<()> {
    match safe_session_directory_exists(directory)? {
        true => {}
        false => return Ok(()),
    }
    let marker = directory.join(PRESERVE_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.is_file() && !crate::app::upload_is_link_like(&metadata) => {
            std::fs::remove_file(marker)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "recovery manifest is not a safe regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn atomic_write_manifest(directory: &Path, contents: &[u8]) -> io::Result<()> {
    // Publish only a completely flushed sibling. Platform replacement keeps
    // readers on either the previous complete snapshot or the new one.
    require_safe_session_directory(directory)?;
    let marker = directory.join(PRESERVE_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if !metadata.is_file() || crate::app::upload_is_link_like(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "recovery manifest is not a safe regular file",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut last_collision = None;
    for _ in 0..16 {
        let sequence = MANIFEST_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = directory.join(format!(
            ".{PRESERVE_MARKER}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        let write_result = file
            .write_all(contents)
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_all());
        drop(file);
        let result = write_result.and_then(|_| replace_file_atomic(&temporary, &marker));
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        return result;
    }
    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "recovery manifest temporary filename collision",
        )
    }))
}

fn safe_session_directory_exists(directory: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() && !crate::app::upload_is_link_like(&metadata) => {
            Ok(true)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "recovery session is not a safe directory: {}",
                directory.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn require_safe_session_directory(directory: &Path) -> io::Result<()> {
    if safe_session_directory_exists(directory)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("recovery session does not exist: {}", directory.display()),
        ))
    }
}

#[cfg(test)]
#[path = "recovery_manifest_task_tests.rs"]
mod task_tests;
