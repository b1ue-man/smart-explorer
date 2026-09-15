use super::api::not_found;
use super::core::{cloud_urlenc, norm, parse_rfc3339_ms, split_parent};
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io;

impl GDriveBackend {
    /// Resolve a forward-slash path to a Drive fileId (walking + caching).
    pub(super) fn resolve(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        if let Some(id) = self.valid_cached_id(&key)? {
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
            if let Some(id) = self.valid_cached_id(&next_path)? {
                cur_id = id;
                cur_path = next_path;
                continue;
            }
            let child = self
                .find_child(&cur_id, seg)?
                .ok_or_else(|| not_found(&next_path))?;
            self.remember_path(&next_path, &child, None)?;
            self.persist_path_cache();
            cur_id = child;
            cur_path = next_path;
        }
        Ok(cur_id)
    }

    pub(super) fn valid_cached_id(&self, key: &str) -> VfsResult<Option<String>> {
        let Some(id) = self.cached_id(key)? else {
            return Ok(None);
        };
        if self.cached_id_is_trusted(key)? {
            return Ok(Some(id));
        }
        if self.validate_cached_id(key, &id)? {
            self.trust_cached_id(key)?;
            self.persist_path_cache();
            return Ok(Some(id));
        }
        self.forget_path_prefix(key);
        Ok(None)
    }

    fn validate_cached_id(&self, key: &str, id: &str) -> VfsResult<bool> {
        if key.is_empty() {
            return Ok(true);
        }
        let (parent, name) = split_parent(key);
        let parent_id = if parent.is_empty() {
            "root".to_string()
        } else {
            match self.resolve(&parent) {
                Ok(id) => id,
                Err(_) => return Ok(false),
            }
        };
        let url = self.api_url(&format!(
            "files/{}?fields=id,name,parents,mimeType,trashed", cloud_urlenc(id)
        ));
        let v = match self.get_json(&url) {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        let matches = if let Some((plain, prefix)) = super::duplicates::parse_marker(name) {
            id.starts_with(prefix)
                && super::cache::validation_matches(&v, &super::names::decode(plain)?, &parent_id)
        } else {
            super::cache::validation_matches(&v, &super::names::decode(name)?, &parent_id)
        };
        if !matches {
            return Ok(false);
        }
        if let Some(mime) = v["mimeType"].as_str() {
            self.mimes_guard()?
                .insert(key.to_string(), mime.to_string());
        }
        Ok(true)
    }

    /// Resolve an encoded segment, with an optional exact sibling marker.
    pub(super) fn find_child(&self, parent_id: &str, segment: &str) -> VfsResult<Option<String>> {
        if let Some((plain, prefix)) = super::duplicates::parse_marker(segment) {
            let siblings = self.same_name_siblings(parent_id, &super::names::decode(plain)?)?;
            return super::duplicates::select_by_prefix(&siblings, prefix)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
        let siblings = self.same_name_siblings(parent_id, &super::names::decode(segment)?)?;
        Ok(super::duplicates::select_canonical(&siblings))
    }

    pub(super) fn same_name_siblings(
        &self,
        parent_id: &str,
        name: &str,
    ) -> VfsResult<Vec<super::duplicates::Sibling>> {
        let quote = |text: &str| text.replace('\\', "\\\\").replace('\'', "\\'");
        let query = format!(
            "'{}' in parents and name = '{}' and trashed = false",
            quote(parent_id), quote(name)
        );
        let mut siblings = Vec::new();
        let mut token: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1_000 {
            let mut url = self.api_url(&format!(
                "files?q={}&fields=nextPageToken,incompleteSearch,files(id,name,modifiedTime)&pageSize=1000",
                cloud_urlenc(&query)
            ));
            if let Some(token) = &token {
                url.push_str(&format!("&pageToken={}", cloud_urlenc(token)));
            }
            let json = self.get_json(&url)?;
            if json["incompleteSearch"].as_bool() == Some(true) {
                return Err(io::Error::other("Drive returned an incomplete name query"));
            }
            let files = json["files"].as_array()
                .ok_or_else(|| io::Error::other("Drive name query has no files array"))?;
            for file in files {
                let id = file["id"].as_str().filter(|id| !id.is_empty())
                    .ok_or_else(|| io::Error::other("Drive name query has no object ID"))?;
                if file["name"].as_str() != Some(name) || !ids.insert(id.to_string()) {
                    return Err(io::Error::other("Drive name query returned inconsistent siblings"));
                }
                siblings.push(super::duplicates::Sibling {
                    id: id.to_string(),
                    mtime_ms: file["modifiedTime"].as_str().and_then(parse_rfc3339_ms).unwrap_or(0),
                });
            }
            token = json["nextPageToken"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
            let Some(next) = &token else { return Ok(siblings); };
            if !seen.insert(next.clone()) {
                return Err(io::Error::other("Drive repeated a name-query page token"));
            }
        }
        Err(io::Error::other("Drive name query exceeded its page budget"))
    }
}
