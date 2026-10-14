//! The user's attachable sessions: `<app data>/shell-sessions/<id>.json`
//! (attach port and token, name, owner, target, dialect) next to
//! `<id>.lock`, which the owner holds while the session lives. An entry whose
//! lock nobody holds is stale and removed by the next listing. Directory and
//! files are private to the user (Unix 0700/0600 with owner and link checks,
//! Windows owner-only DACL), so the attach token reaches only the user's own
//! processes.
use super::types::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::File;
use std::path::{Path, PathBuf};

const MAX_ENTRY_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::cli) struct Entry {
    pub(in crate::cli) id: String,
    pub(in crate::cli) name: String,
    pub(in crate::cli) owner: String,
    pub(in crate::cli) target: String,
    pub(in crate::cli) dialect: Dialect,
    pub(super) port: u16,
    pub(super) token: String,
    pub(super) created_ms: i64,
}

impl Entry {
    /// What listings show: everything except the attach port and token.
    pub(in crate::cli) fn public_value(&self) -> Value {
        json!({
            "id": self.id,
            "session": self.name,
            "owner": self.owner,
            "target": self.target,
            "dialect": self.dialect,
            "created_ms": self.created_ms,
        })
    }
}

/// Keeps an entry listed; dropping it removes the entry.
pub(super) struct Registration {
    lock: Option<File>,
    json: PathBuf,
    lock_path: PathBuf,
}

impl Drop for Registration {
    fn drop(&mut self) {
        drop(self.lock.take());
        let _ = std::fs::remove_file(&self.json);
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

pub(super) fn directory() -> PathBuf {
    crate::support_dirs::app_data_dir().join("shell-sessions")
}

/// Live attachable sessions, oldest first.
pub(in crate::cli) fn list() -> Vec<Entry> {
    list_in(&directory())
}

/// The live session named `selector`, or with that id.
pub(in crate::cli) fn find(selector: &str) -> Result<Entry, String> {
    find_in(&directory(), selector)
}

pub(super) fn register_in(directory: &Path, entry: &Entry) -> Result<Registration, String> {
    crate::support_dirs::ensure_private_dir(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let lock_path = directory.join(format!("{}.lock", entry.id));
    let lock = crate::support_dirs::open_private_lock(&lock_path)
        .map_err(|error| format!("cannot create the session lock: {error}"))?;
    lock.try_lock()
        .map_err(|error| format!("cannot lock the session entry: {error}"))?;
    let json = directory.join(format!("{}.json", entry.id));
    let registration = Registration {
        lock: Some(lock),
        json: json.clone(),
        lock_path,
    };
    let bytes = serde_json::to_vec(entry).map_err(|error| error.to_string())?;
    crate::support_dirs::write_private_atomic(&json, &bytes)
        .map_err(|error| format!("cannot record the attachable session: {error}"))?;
    Ok(registration)
}

pub(super) fn list_in(directory: &Path) -> Vec<Entry> {
    let Ok(items) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for item in items.flatten() {
        let path = item.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
            .map(str::to_string)
        else {
            continue;
        };
        let lock_path = directory.join(format!("{id}.lock"));
        if !held(&lock_path) {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&lock_path);
            continue;
        }
        let Ok(text) = crate::support_dirs::read_private_text(&path, MAX_ENTRY_BYTES) else {
            continue;
        };
        if let Ok(entry) = serde_json::from_str::<Entry>(&text) {
            if entry.id == id {
                entries.push(entry);
            }
        }
    }
    entries.sort_by(|left, right| {
        left.created_ms
            .cmp(&right.created_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    entries
}

pub(super) fn find_in(directory: &Path, selector: &str) -> Result<Entry, String> {
    let selector = selector.trim();
    let entries = list_in(directory);
    let matches: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.id == selector || entry.name == selector)
        .collect();
    match matches.as_slice() {
        [entry] => Ok((*entry).clone()),
        [] => Err(format!(
            "no attachable session named {selector:?}; `se shell --list` shows the attachable sessions (a `se shell` needs --attachable)"
        )),
        several => Err(format!(
            "several sessions are named {selector:?}; use the id: {}",
            several
                .iter()
                .map(|entry| format!("{} ({}, {})", entry.id, entry.owner, entry.target))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Whether the owner still holds the entry's lock. Only a missing lock file
/// or a lock that can be taken proves the entry stale.
fn held(lock_path: &Path) -> bool {
    match File::options().read(true).write(true).open(lock_path) {
        Ok(lock) => lock.try_lock().is_err(),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::{find_in, list_in, register_in, Entry};
    use crate::cli::shell_session::types::Dialect;

    fn entry(id: &str, name: &str, created_ms: i64) -> Entry {
        Entry {
            id: id.into(),
            name: name.into(),
            owner: "se shell (pid 1)".into(),
            target: "@prod".into(),
            dialect: Dialect::Posix,
            port: 4000,
            token: "ab".repeat(32),
            created_ms,
        }
    }

    #[test]
    fn shell_session_task_registry_lists_live_entries_and_removes_stale_ones() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("shell-sessions");
        let live = entry("00000000000000aa", "build", 2);
        let registration = register_in(&root, &live).unwrap();
        // Stale: its lock file exists, but nobody holds it.
        let stale = entry("00000000000000bb", "old", 1);
        std::fs::write(
            root.join("00000000000000bb.json"),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("00000000000000bb.lock"), b"").unwrap();
        assert_eq!(list_in(&root), vec![live.clone()]);
        assert!(!root.join("00000000000000bb.json").exists());
        assert_eq!(find_in(&root, "build").unwrap().id, live.id);
        assert_eq!(find_in(&root, &live.id).unwrap().name, "build");
        assert!(find_in(&root, "old").is_err());
        let public = live.public_value();
        assert!(public.get("token").is_none() && public.get("port").is_none());
        let twin = entry("00000000000000cc", "build", 3);
        let second = register_in(&root, &twin).unwrap();
        assert!(find_in(&root, "build")
            .unwrap_err()
            .contains("00000000000000cc"));
        drop(second);
        drop(registration);
        assert!(list_in(&root).is_empty());
        assert!(!root.join("00000000000000aa.json").exists());
    }
}
