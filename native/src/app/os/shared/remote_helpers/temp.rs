use super::recovery_manifest::{stale_temp_disposition, StaleTempDisposition};
use crate::app::app_models::TEMP_SESSION_PID_FILE;
use crate::app::platform_helpers::EditProcess;
use std::path::{Path, PathBuf};

/// Root for all of this app's open/edit temp copies.
pub(in crate::app) fn temp_root() -> PathBuf {
    crate::support_dirs::app_data_dir().join("open-temp")
}

pub(super) const PRESERVE_MARKER: &str = "preserved-recovery.txt";

/// A stable tag unique to THIS process run (`<pid>_<start-nanos>`), so we can
/// tell our current session's temp dirs from stale ones left by prior runs.
pub(in crate::app) fn session_tag() -> &'static str {
    static T: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("s{}_{}", std::process::id(), nanos)
    })
}

pub(in crate::app) fn session_temp_dir() -> PathBuf {
    temp_root().join(session_tag())
}

pub(in crate::app) fn session_marker_path(dir: &Path) -> PathBuf {
    dir.join(TEMP_SESSION_PID_FILE)
}

pub(in crate::app) fn init_temp_session() -> usize {
    let recoverable_sessions = sweep_stale_temp();
    let _ = write_session_marker();
    recoverable_sessions
}

pub(in crate::app) fn write_session_marker() -> std::io::Result<()> {
    let dir = session_temp_dir();
    std::fs::create_dir_all(&dir)?;
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::fs::write(
        session_marker_path(&dir),
        format!(
            "pid={}\ntag={}\nstarted_ms={}\n",
            std::process::id(),
            session_tag(),
            started
        ),
    )
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

pub(in crate::app) fn safe_temp_name(name: &str) -> String {
    let safe = name.replace(['/', '\\', ':'], "_");
    if safe.trim().is_empty() {
        "datei".to_string()
    } else {
        safe
    }
}

/// A fresh, unique local path to download a remote file to for opening or
/// editing. Each call gets its own temp subdirectory.
pub(in crate::app) fn open_temp_path(name: &str) -> std::io::Result<PathBuf> {
    write_session_marker()?;
    allocate_open_temp_path(&session_temp_dir(), name)
}

fn allocate_open_temp_path(root: &Path, name: &str) -> std::io::Result<PathBuf> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::fs::create_dir_all(root)?;
    let safe = safe_temp_name(name);
    for _ in 0..16 {
        let mut bytes = [0u8; 8];
        if getrandom::getrandom(&mut bytes).is_err() {
            break;
        }
        let dir = root.join(format!("e{:016x}", u64::from_le_bytes(bytes)));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir.join(&safe)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    for _ in 0..16 {
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = root.join(format!("e{}_{}", std::process::id(), n));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir.join(&safe)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "Kein eindeutiges temporäres Verzeichnis verfügbar",
    ))
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

/// Delete this session's temp copies on a clean exit.
pub(in crate::app) fn cleanup_session_temp() {
    let _ = super::temp_delete::remove_owned_tree(&temp_root(), &session_temp_dir());
}

pub(in crate::app) fn cleanup_temp_copy(temp: &Path) {
    if let Some(parent) = temp.parent() {
        if parent.starts_with(session_temp_dir()) {
            let _ = super::temp_delete::remove_owned_tree(&temp_root(), parent);
            return;
        }
    }
    let _ = std::fs::remove_file(temp);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "smart_explorer_open_temp_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn allocation_creates_a_unique_parent_and_sanitizes_the_name() {
        let root = test_root("success");
        let path = allocate_open_temp_path(&root, "a/b:c.txt").unwrap();

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("a_b_c.txt")
        );
        assert!(path.parent().is_some_and(Path::is_dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn allocation_propagates_directory_creation_failure() {
        let parent = test_root("failure");
        std::fs::create_dir_all(&parent).unwrap();
        let blocker = parent.join("not-a-directory");
        std::fs::write(&blocker, b"file").unwrap();

        let result = allocate_open_temp_path(&blocker.join("child"), "file.txt");

        assert!(result.is_err());
        assert!(!blocker.join("child").exists());
        std::fs::remove_dir_all(parent).unwrap();
    }
}
