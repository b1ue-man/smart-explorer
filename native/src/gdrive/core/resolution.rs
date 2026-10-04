use super::api::{not_found, FOLDER_MIME};
use super::core::{norm, parse_rfc3339_ms, split_parent};
use super::identity::{in_parent, invalid, text};
use super::overload::http_status;
use super::GDriveBackend;
use crate::vfs::VfsResult;
use serde_json::Value;
use std::io;

impl GDriveBackend {
    /// Walk through immutable folder bindings before consulting path hints.
    /// An unavailable or missing known folder never falls back to a sibling.
    pub(super) fn resolve(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        let mut current = self.actual_parent_id("root")?;
        let mut current_path = String::new();
        for segment in key.split('/').filter(|segment| !segment.is_empty()) {
            let next = super::sync_projection::child_key(&current_path, segment);
            if let Some(id) = self.resolve_bound_folder(&current, segment)? {
                self.remember_path(&next, &id, Some(FOLDER_MIME))?;
                current = id;
                current_path = next;
                continue;
            }
            if let Some(id) = self.valid_cached_id(&next)? {
                current = id;
                current_path = next;
                continue;
            }
            // An old root cannot prove a historical choice from several
            // folders. A complete parent may durably bind its initial choice.
            let root_hop = next != key || (!self.root.is_empty()
                && (self.root == next || self.root.starts_with(&format!("{next}/"))));
            let child = self.child_object(&current, segment, root_hop)?
                .ok_or_else(|| not_found(&next))?;
            let id = text(&child, "id")?.to_string();
            let mime = text(&child, "mimeType")?;
            if mime == FOLDER_MIME {
                let title = text(&child, "name")?;
                self.folder_evidence(&current, title, &id)?
                    .ok_or_else(|| not_found(&next))?;
                self.bind_folder_path(&current, title, &id, segment)?;
            }
            self.remember_path(&next, &id, Some(mime))?;
            self.persist_path_cache();
            current = id;
            current_path = next;
        }
        Ok(current)
    }

    pub(super) fn valid_cached_id(&self, key: &str) -> VfsResult<Option<String>> {
        let Some(id) = self.cached_id(key)? else { return Ok(None); };
        if key.is_empty() { return self.actual_parent_id(&id).map(Some); }
        let (parent_path, segment) = split_parent(key);
        let parent = self.resolve(&parent_path)?;
        // Legacy upload callers consult this method without walking the
        // child first. They must observe reserved or unreadable registry state
        // even when an ordinary file hint is already trusted.
        if let Some(bound) = self.resolve_bound_folder(&parent, segment)? {
            return Ok(Some(bound));
        }
        let folder_hint = self.mimes_guard()?.get(key).is_some_and(|mime| mime == FOLDER_MIME);
        // A trusted legacy folder hint must become durable before use.
        if self.cached_id_is_trusted(key)? && !folder_hint { return Ok(Some(id)); }
        if self.validate_cached_id(key, &id, &parent, segment)? {
            self.trust_cached_id(key)?;
            self.persist_path_cache();
            return Ok(Some(id));
        }
        if folder_hint {
            return Err(not_found(&format!("previous Drive folder {key} ({id})")));
        }
        self.forget_path_prefix(key);
        Ok(None)
    }

    fn validate_cached_id(&self, key: &str, id: &str, parent: &str, segment: &str) -> VfsResult<bool> {
        let json = match self.object_json(id) {
            Ok(json) => json,
            Err(error) if http_status(&error) == Some(404) => return Ok(false),
            Err(error) => return Err(error),
        };
        let title = match super::duplicates::parse_marker(segment) {
            Some((plain, prefix)) => {
                if !id.starts_with(prefix) { return Ok(false); }
                super::names::decode(plain)?
            }
            None => super::names::decode(segment)?,
        };
        if !in_parent(&json, parent)? || text(&json, "name")? != title { return Ok(false); }
        let mime = text(&json, "mimeType")?;
        if mime == FOLDER_MIME {
            self.bind_folder_path(parent, &title, id, segment)?;
        }
        self.mimes_guard()?.insert(key.to_string(), mime.to_string());
        Ok(true)
    }

    pub(super) fn find_child(&self, parent: &str, segment: &str) -> VfsResult<Option<String>> {
        if let Some(id) = self.resolve_bound_folder(parent, segment)? { return Ok(Some(id)); }
        self.child_object(parent, segment, false)?
            .map(|json| text(&json, "id").map(str::to_string)).transpose()
    }

    pub(super) fn find_folder_child(&self, parent: &str, segment: &str) -> VfsResult<Option<String>> {
        if let Some(id) = self.resolve_bound_folder(parent, segment)? { return Ok(Some(id)); }
        let Some(child) = self.child_object(parent, segment, true)? else { return Ok(None); };
        if text(&child, "mimeType")? != FOLDER_MIME {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Drive directory path names a file"));
        }
        let id = text(&child, "id")?.to_string();
        let parent = self.actual_parent_id(parent)?;
        let title = text(&child, "name")?;
        self.folder_evidence(&parent, title, &id)?
            .ok_or_else(|| not_found(segment))?;
        self.bind_folder_path(&parent, title, &id, segment)?;
        Ok(Some(id))
    }

    fn child_object(&self, parent: &str, segment: &str, require_folder_choice: bool) -> VfsResult<Option<Value>> {
        let marker = super::duplicates::parse_marker(segment);
        let title = super::names::decode(marker.map(|(plain, _)| plain).unwrap_or(segment))?;
        let mut children = self.collect_files(parent, Some(&title))?;
        if let Some((_, prefix)) = marker {
            let id = super::duplicates::select_by_prefix(&siblings(&children), prefix).map_err(invalid)?;
            return Ok(id.and_then(|id| children.into_iter().find(|child| child["id"].as_str() == Some(id.as_str()))));
        }
        let registry = self.binding_store.read(&self.drive_account_key, &self.actual_parent_id(parent)?)?;
        children.retain(|child| child["mimeType"].as_str() != Some(FOLDER_MIME)
            || registry.by_object(&title, child["id"].as_str().unwrap_or_default()).is_none());
        if require_folder_choice {
            let folders: Vec<_> = children.iter().filter(|child| child["mimeType"].as_str() == Some(FOLDER_MIME)).collect();
            if folders.len() > 1 {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists,
                    "Drive folder path has several identities; select the exact folder in the existing picker"));
            }
            if let Some(folder) = folders.first() { return Ok(Some((**folder).clone())); }
        } else if children.iter().any(|child| child["mimeType"].as_str() != Some(FOLDER_MIME)) {
            // A mixed name's folders are projected separately. The literal
            // file locator continues to address the file variant group.
            children.retain(|child| child["mimeType"].as_str() != Some(FOLDER_MIME));
        }
        let id = super::duplicates::select_canonical(&siblings(&children));
        Ok(id.and_then(|id| children.drain(..).find(|child| child["id"].as_str() == Some(id.as_str()))))
    }

    pub(super) fn same_name_siblings(&self, parent: &str, name: &str) -> VfsResult<Vec<super::duplicates::Sibling>> {
        self.collect_files(parent, Some(name)).map(|files| siblings(&files))
    }
}

fn siblings(files: &[Value]) -> Vec<super::duplicates::Sibling> {
    files.iter().map(|file| super::duplicates::Sibling {
        id: file["id"].as_str().unwrap_or_default().to_string(),
        mtime_ms: file["modifiedTime"].as_str().and_then(parse_rfc3339_ms).unwrap_or(0),
    }).collect()
}
