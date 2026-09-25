//! Remote files opened for editing and the startup sweep of earlier sessions.
//! Temp-copy allocation and cleanup live in `crate::transfer`.
use super::recovery_manifest::{stale_temp_disposition, StaleTempDisposition};
use crate::app::platform_helpers::EditProcess;
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(in crate::app) use crate::transfer::safe_temp_name;
pub(in crate::app) use crate::transfer::{
    cleanup_session_temp, cleanup_temp_copy, open_temp_path, session_tag, session_temp_dir,
    temp_root,
};
use crate::transfer::{session_marker_path, write_session_marker};

pub(super) const PRESERVE_MARKER: &str = "preserved-recovery.txt";

pub(in crate::app) fn init_temp_session() -> usize {
    let recoverable_sessions = sweep_stale_temp();
    let _ = write_session_marker();
    recoverable_sessions
}

pub(in crate::app) fn read_session_pid(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(session_marker_path(dir)).ok()?;
    for line in text.lines() {
        if let Some(pid) = line
            .strip_prefix("pid=")
            .and_then(|s| s.trim().parse().ok())
        {
            return Some(pid);
        }
        if let Ok(pid) = line.trim().parse() {
            return Some(pid);
        }
    }
    None
}

/// Remove leftover temp copies from previous sessions.
pub(in crate::app) fn sweep_stale_temp() -> usize {
    let mut recoverable = 0usize;
    if let Ok(rd) = std::fs::read_dir(temp_root()) {
        for e in rd.flatten() {
            match stale_temp_disposition(&e.path()) {
                StaleTempDisposition::Cleanup => {
                    let _ = super::temp_delete::remove_owned_tree(&temp_root(), &e.path());
                }
                StaleTempDisposition::Recovery => {
                    recoverable = recoverable.saturating_add(1);
                }
                StaleTempDisposition::Ignore => {}
            }
        }
    }
    recoverable
}

pub(in crate::app) fn file_mtime_ms(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A remote file opened for editing in temp mode.
pub(in crate::app) struct RemoteEdit {
    pub(in crate::app) temp: PathBuf,
    pub(in crate::app) backend: crate::vfs::BackendHandle,
    pub(in crate::app) remote_path: String,
    pub(in crate::app) name: String,
    /// Last mtime uploaded/downloaded: a change above this is a save.
    pub(in crate::app) baseline_mtime: i64,
    /// mtime seen last poll.
    pub(in crate::app) seen_mtime: i64,
    /// The remote file's mtime when we last synced it.
    pub(in crate::app) remote_known_mtime: i64,
    pub(in crate::app) dirty: bool,
    pub(in crate::app) uploading: bool,
    pub(in crate::app) process: Option<EditProcess>,
}

/// Outcome of a save-back upload attempt.
pub(in crate::app) enum SaveResult {
    /// Uploaded; carries the remote's new mtime to re-baseline against.
    Ok(i64),
    /// The remote changed since we downloaded it and was not overwritten.
    Conflict(i64),
    /// Upload failed.
    Failed(String),
}
