use super::GDriveBackend;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

const CACHE_VERSION: u32 = 2;
const CACHE_FILE: &str = "path_cache.json";

#[derive(Default)]
pub(super) struct LoadedCache {
    pub ids: HashMap<String, String>,
    pub mimes: HashMap<String, String>,
}

pub(super) struct LoadedCaches {
    pub hints: LoadedCache,
    pub historical_ids: HashMap<String, String>,
}

#[derive(Deserialize, Serialize)]
struct DiskCache {
    version: u32,
    ids: HashMap<String, String>,
    mimes: HashMap<String, String>,
}

pub(super) fn cache_path() -> PathBuf {
    super::binding_store::legacy_cache_path()
}

/// Opening an unrelated root can already have written an account cache.
/// Keep the still-existing global v2 hints independently reachable; only
/// those bytes establish historical origin, never the account cache alone.
pub(super) fn load_account(account_path: &Path, legacy_path: &Path) -> LoadedCaches {
    let historical = load_from_path(legacy_path).unwrap_or_default();
    let mut hints = load_from_path(account_path).unwrap_or_default();
    for (key, id) in &historical.ids {
        hints.ids.insert(key.clone(), id.clone());
        match historical.mimes.get(key) {
            Some(mime) => {
                hints.mimes.insert(key.clone(), mime.clone());
            }
            None => {
                hints.mimes.remove(key);
            }
        }
    }
    LoadedCaches {
        hints,
        historical_ids: historical.ids,
    }
}

pub(super) fn load_from_path(path: &Path) -> io::Result<LoadedCache> {
    let text = super::binding_store::read_hint_cache(path)?;
    let disk: DiskCache = serde_json::from_str(&text).map_err(io::Error::other)?;
    if disk.version != CACHE_VERSION {
        return Ok(LoadedCache::default());
    }
    Ok(LoadedCache {
        ids: clean_map(disk.ids),
        mimes: clean_map(disk.mimes),
    })
}

/// Keep the v2 hint format; the host adapter owns private durable replacement.
pub(super) fn save_to_path(
    path: &Path,
    ids: HashMap<String, String>,
    mimes: HashMap<String, String>,
) -> io::Result<()> {
    let disk = DiskCache {
        version: CACHE_VERSION,
        ids: clean_map(ids),
        mimes: clean_map(mimes),
    };
    let bytes = serde_json::to_vec(&disk).map_err(io::Error::other)?;
    super::binding_store::write_hint_cache(path, &bytes)
}

fn clean_map(mut map: HashMap<String, String>) -> HashMap<String, String> {
    map.retain(|k, v| !k.is_empty() && !v.is_empty());
    map
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{}/", prefix.trim_end_matches('/')))
}

pub(super) fn validation_matches(
    v: &serde_json::Value,
    expected_name: &str,
    expected_parent_id: &str,
) -> bool {
    if v["trashed"].as_bool().unwrap_or(false) {
        return false;
    }
    if v["name"].as_str() != Some(expected_name) {
        return false;
    }
    v["parents"].as_array().is_some_and(|parents| {
        parents
            .iter()
            .any(|p| p.as_str() == Some(expected_parent_id))
    })
}

impl GDriveBackend {
    pub(super) fn cached_id(&self, key: &str) -> io::Result<Option<String>> {
        Ok(self.ids_guard()?.get(key).cloned())
    }

    pub(super) fn cached_id_is_trusted(&self, key: &str) -> io::Result<bool> {
        Ok(!self.untrusted_guard()?.contains(key))
    }

    pub(super) fn trust_cached_id(&self, key: &str) -> io::Result<()> {
        self.untrusted_guard()?.remove(key);
        Ok(())
    }

    pub(super) fn remember_path(&self, key: &str, id: &str, mime: Option<&str>) -> io::Result<()> {
        let previous = self.ids_guard()?.insert(key.to_string(), id.to_string());
        match mime.filter(|m| !m.is_empty()) {
            Some(mime) => {
                self.mimes_guard()?
                    .insert(key.to_string(), mime.to_string());
            }
            // A MIME type learned for another object at this path would be
            // stale now (a download could pick the wrong export format).
            None if previous.as_deref().is_some_and(|previous| previous != id) => {
                self.mimes_guard()?.remove(key);
            }
            None => {}
        }
        self.untrusted_guard()?.remove(key);
        Ok(())
    }

    pub(super) fn forget_path_prefix(&self, prefix: &str) {
        let prefix = super::core::norm(prefix);
        if prefix.is_empty() {
            return;
        }
        if let Ok(mut ids) = self.ids_guard() {
            remove_prefix(&mut ids, &prefix);
        }
        if let Ok(mut mimes) = self.mimes_guard() {
            remove_prefix(&mut mimes, &prefix);
        }
        if let Ok(mut untrusted) = self.untrusted_guard() {
            untrusted.retain(|path| !path_matches_prefix(path, &prefix));
        }
        self.persist_path_cache();
    }

    /// Mark the path cache changed; the background writer saves it soon.
    pub(super) fn persist_path_cache(&self) {
        self.cache_store.mark_dirty();
    }

    /// Save the path cache before returning (folder-journal ordering).
    pub(super) fn persist_path_cache_checked(&self) -> io::Result<()> {
        self.cache_store.write_now()
    }
}

fn remove_prefix(map: &mut HashMap<String, String>, prefix: &str) {
    map.retain(|path, _| !path_matches_prefix(path, prefix));
}

pub(super) fn loaded_untrusted(ids: &HashMap<String, String>) -> HashSet<String> {
    ids.keys().filter(|k| !k.is_empty()).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!(
            "se_gdrive_cache_{tag}_{}_{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&p).unwrap();
        p.join(CACHE_FILE)
    }

    #[test]
    fn load_save_roundtrip_excludes_root_and_empty_values() {
        let path = tmp_file("roundtrip");
        let mut ids = HashMap::from([
            ("".to_string(), "root".to_string()),
            ("docs".to_string(), "id-docs".to_string()),
            ("empty".to_string(), String::new()),
        ]);
        let mimes = HashMap::from([
            ("docs/a.txt".to_string(), "text/plain".to_string()),
            ("".to_string(), "ignored".to_string()),
        ]);
        save_to_path(&path, ids.clone(), mimes).unwrap();
        ids.clear();
        let loaded = load_from_path(&path).unwrap();
        assert_eq!(loaded.ids.get("docs").map(String::as_str), Some("id-docs"));
        assert!(!loaded.ids.contains_key(""));
        assert!(!loaded.ids.contains_key("empty"));
        assert_eq!(
            loaded.mimes.get("docs/a.txt").map(String::as_str),
            Some("text/plain")
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn corrupt_cache_is_ignored_by_public_loader_shape() {
        let path = tmp_file("corrupt");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_from_path(&path).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn prefix_removal_keeps_sibling_paths() {
        let mut ids = HashMap::from([
            ("docs".to_string(), "id-docs".to_string()),
            ("docs/a.txt".to_string(), "id-a".to_string()),
            ("docs2/a.txt".to_string(), "id-b".to_string()),
        ]);
        remove_prefix(&mut ids, "docs");
        assert!(!ids.contains_key("docs"));
        assert!(!ids.contains_key("docs/a.txt"));
        assert!(ids.contains_key("docs2/a.txt"));
    }

    #[test]
    fn validation_rejects_stale_or_trashed_ids() {
        let good = serde_json::json!({
            "name": "a.txt",
            "parents": ["parent"],
            "trashed": false
        });
        let wrong_name = serde_json::json!({
            "name": "b.txt",
            "parents": ["parent"],
            "trashed": false
        });
        let trashed = serde_json::json!({
            "name": "a.txt",
            "parents": ["parent"],
            "trashed": true
        });
        assert!(validation_matches(&good, "a.txt", "parent"));
        assert!(!validation_matches(&wrong_name, "a.txt", "parent"));
        assert!(!validation_matches(&trashed, "a.txt", "parent"));
    }

    #[test]
    fn loaded_ids_start_untrusted_without_listed_absence_state() {
        let ids = HashMap::from([("docs".to_string(), "id-docs".to_string())]);
        let untrusted = loaded_untrusted(&ids);
        assert!(untrusted.contains("docs"));
        assert_eq!(untrusted.len(), 1);
    }
}
