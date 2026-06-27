use super::api::{drive_request, err, not_found, parse_json, send_retry, API, FOLDER_MIME};
use super::core::{cloud_urlenc, norm, parse_rfc3339_ms, split_parent};
use super::GDriveBackend;
use crate::vfs::{VfsMeta, VfsResult};

impl GDriveBackend {
    /// The Drive mimeType for `path` (cached from list_dir, else a stat call).
    pub(super) fn mime_of(&self, path: &str) -> Option<String> {
        let key = norm(path);
        if let Some(m) = self.mimes_guard().ok()?.get(&key).cloned() {
            return Some(m);
        }
        let id = self.resolve(&key).ok()?;
        let url = format!("{}/files/{}?fields=mimeType", API, id);
        let v = self.get_json(&url).ok()?;
        let m = v["mimeType"].as_str()?.to_string();
        self.mimes_guard().ok()?.insert(key, m.clone());
        Some(m)
    }

    /// Resolve a forward-slash path to a Drive fileId (walking + caching).
    pub(super) fn resolve(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        if let Some(id) = self.ids_guard()?.get(&key).cloned() {
            return Ok(id);
        }
        // Walk segment by segment from the deepest cached ancestor.
        let segs: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
        let mut cur_id = "root".to_string();
        let mut cur_path = String::new();
        for seg in segs {
            let next_path = if cur_path.is_empty() {
                seg.to_string()
            } else {
                format!("{}/{}", cur_path, seg)
            };
            if let Some(id) = self.ids_guard()?.get(&next_path).cloned() {
                cur_id = id;
                cur_path = next_path;
                continue;
            }
            let child = self
                .find_child(&cur_id, seg)?
                .ok_or_else(|| not_found(&next_path))?;
            self.ids_guard()?.insert(next_path.clone(), child.clone());
            cur_id = child;
            cur_path = next_path;
        }
        Ok(cur_id)
    }

    pub(super) fn find_child(&self, parent_id: &str, name: &str) -> VfsResult<Option<String>> {
        let q = format!(
            "'{}' in parents and name = '{}' and trashed = false",
            parent_id,
            name.replace('\'', "\\'")
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name)&pageSize=1",
            API,
            cloud_urlenc(&q)
        );
        let v = self.get_json(&url)?;
        Ok(v["files"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|f| f["id"].as_str())
            .map(|s| s.to_string()))
    }

    pub(super) fn meta_from_json(
        f: &serde_json::Value,
        fallback_name: Option<&str>,
    ) -> Option<VfsMeta> {
        let is_dir = f["mimeType"].as_str() == Some(FOLDER_MIME);
        let name = f["name"]
            .as_str()
            .or(fallback_name)
            .filter(|name| !name.is_empty())?;
        Some(VfsMeta {
            name: name.to_string(),
            is_dir,
            is_symlink: false,
            size: f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
            mtime_ms: f["modifiedTime"]
                .as_str()
                .and_then(parse_rfc3339_ms)
                .unwrap_or(0),
            btime_ms: f["createdTime"]
                .as_str()
                .and_then(parse_rfc3339_ms)
                .unwrap_or(0),
            hidden: false,
            system: false,
            id: f["id"].as_str().map(|s| s.to_string()),
            content_md5: f["md5Checksum"].as_str().map(|s| s.to_string()),
        })
    }

    /// MIME type of a file by its id (for export detection when opening by id).
    pub(super) fn mime_of_id(&self, id: &str) -> Option<String> {
        let url = format!("{}/files/{}?fields=mimeType", API, id);
        self.get_json(&url).ok()?["mimeType"]
            .as_str()
            .map(|s| s.to_string())
    }

    /// Ensure a folder path exists, returning the deepest folder's id.
    /// Thread-safe: concurrent transfers may need the same folder, so the
    /// find-or-create of each level is serialized (parents are resolved first,
    /// outside the lock, to avoid re-entrancy).
    pub(super) fn ensure_dir(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        if key.is_empty() {
            return Ok("root".to_string());
        }
        if let Some(id) = self.ids_guard()?.get(&key).cloned() {
            return Ok(id);
        }
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;

        let _g = self.create_guard()?;
        // Re-check under the lock - another thread may have just created it.
        if let Some(id) = self.ids_guard()?.get(&key).cloned() {
            return Ok(id);
        }
        // If the parent's children are fully known and this folder isn't among
        // them, it's known-absent -> skip the existence query.
        let known_absent = self.listed_guard()?.contains(&parent);
        let existing = if known_absent {
            None
        } else {
            self.find_child(&parent_id, name)?
        };
        if let Some(id) = existing {
            self.ids_guard()?.insert(key, id.clone());
            return Ok(id);
        }
        // Create the folder.
        let body = serde_json::json!({
            "name": name,
            "mimeType": FOLDER_MIME,
            "parents": [parent_id],
        });
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let payload = body.to_string();
        let v = parse_json(send_retry(|| {
            drive_request(
                ureq::post(&format!("{}/files?fields=id", API))
                    .set("Authorization", &bearer)
                    .set("Content-Type", "application/json")
                    .send_string(&payload),
            )
        })?)?;
        let id = v["id"]
            .as_str()
            .ok_or_else(|| err("kein id nach mkdir"))?
            .to_string();
        self.ids_guard()?.insert(key.clone(), id.clone());
        // A brand-new folder has no children -> its contents are fully known.
        self.listed_guard()?.insert(key);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn meta_from_json_requires_a_usable_name() {
        let f = json!({"id": "1", "mimeType": "text/plain"});
        assert!(GDriveBackend::meta_from_json(&f, None).is_none());

        let m = GDriveBackend::meta_from_json(&f, Some("fallback.txt")).unwrap();
        assert_eq!(m.name, "fallback.txt");
    }
}
