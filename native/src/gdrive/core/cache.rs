use super::GDriveBackend;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = "path_cache.json";

#[derive(Default)]
pub(super) struct LoadedCache {
    pub ids: HashMap<String, String>,
    pub mimes: HashMap<String, String>,
}

#[derive(Deserialize, Serialize)]
struct DiskCache {
    version: u32,
    ids: HashMap<String, String>,
    mimes: HashMap<String, String>,
}

pub(super) fn load() -> LoadedCache {
    load_from_path(&cache_path()).unwrap_or_default()
}

fn cache_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("gdrive")
        .join(CACHE_FILE)
}

fn load_from_path(path: &Path) -> io::Result<LoadedCache> {
    let text = std::fs::read_to_string(path)?;
    let disk: DiskCache = serde_json::from_str(&text).map_err(io::Error::other)?;
    if disk.version != CACHE_VERSION {
        return Ok(LoadedCache::default());
    }
    Ok(LoadedCache {
        ids: clean_map(disk.ids),
        mimes: clean_map(disk.mimes),
    })
}

fn save_to_path(
    path: &Path,
    ids: &HashMap<String, String>,
    mimes: &HashMap<String, String>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let disk = DiskCache {
        version: CACHE_VERSION,
        ids: clean_map(ids.clone()),
        mimes: clean_map(mimes.clone()),
    };
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&disk).map_err(io::Error::other)?,
    )?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(first) => {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path).map_err(|_| first)
        }
    }
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
        self.ids_guard()?.insert(key.to_string(), id.to_string());
        if let Some(mime) = mime.filter(|m| !m.is_empty()) {
            self.mimes_guard()?
                .insert(key.to_string(), mime.to_string());
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

    pub(super) fn persist_path_cache(&self) {
        if let (Ok(ids), Ok(mimes)) = (self.ids_guard(), self.mimes_guard()) {
            let _ = save_to_path(&cache_path(), &ids, &mimes);
        }
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
        save_to_path(&path, &ids, &mimes).unwrap();
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
