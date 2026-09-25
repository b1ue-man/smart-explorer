//! App-owned temp copies under `<app data>/open-temp/<session>/e<random>/`:
//! opened remote files, clipboard materializations and remote-copy bridges.
use std::path::{Path, PathBuf};

/// Marker file naming the process that owns a session temp directory.
pub const TEMP_SESSION_PID_FILE: &str = "session.pid";

/// Root for all of this app's open/edit temp copies.
pub fn temp_root() -> PathBuf {
    crate::support_dirs::app_data_dir().join("open-temp")
}

/// A stable tag unique to THIS process run (`<pid>_<start-nanos>`), so we can
/// tell our current session's temp dirs from stale ones left by prior runs.
pub(crate) fn session_tag() -> &'static str {
    static T: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("s{}_{}", std::process::id(), nanos)
    })
}

pub fn session_temp_dir() -> PathBuf {
    temp_root().join(session_tag())
}

pub(crate) fn session_marker_path(dir: &Path) -> PathBuf {
    dir.join(TEMP_SESSION_PID_FILE)
}

pub(crate) fn write_session_marker() -> std::io::Result<()> {
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

pub fn safe_temp_name(name: &str) -> String {
    let safe = name.replace(['/', '\\', ':'], "_");
    if safe.trim().is_empty() {
        "datei".to_string()
    } else {
        safe
    }
}

/// A fresh, unique local path to download a remote file to for opening or
/// editing. Each call gets its own temp subdirectory.
pub fn open_temp_path(name: &str) -> std::io::Result<PathBuf> {
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

/// Delete this session's temp copies on a clean exit.
pub fn cleanup_session_temp() {
    let _ = super::temp_delete::remove_owned_tree(&temp_root(), &session_temp_dir());
}

pub fn cleanup_temp_copy(temp: &Path) {
    if let Some(parent) = temp.parent() {
        if parent.starts_with(session_temp_dir()) {
            let _ = super::temp_delete::remove_owned_tree(&temp_root(), parent);
            return;
        }
    }
    let _ = std::fs::remove_file(temp);
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
