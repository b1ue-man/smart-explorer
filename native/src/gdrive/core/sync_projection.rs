//! Stable sync folder names. Files keep literal names and their captured IDs;
//! only folder trees acquire a registered exact locator and logical alias.
use super::api::not_found;
use super::core::{norm, split_parent};
use super::identity::invalid;
use super::listing::RawEntry;
use super::sync_bindings::FolderBinding;
use super::GDriveBackend;
use std::collections::HashMap;
use std::io;

impl GDriveBackend {
    pub(super) fn bound_folder(&self, parent: &str, segment: &str) -> io::Result<Option<FolderBinding>> {
        let parent = self.actual_parent_id(parent)?;
        Ok(self.binding_store.read(&self.drive_account_key, &parent)?.by_segment(segment).cloned())
    }

    pub(super) fn resolve_bound_folder(&self, parent: &str, segment: &str) -> io::Result<Option<String>> {
        let parent = self.actual_parent_id(parent)?;
        let Some(binding) = self.bound_folder(&parent, segment)? else { return Ok(None); };
        if self.folder_evidence(&parent, &binding.title, &binding.id)?.is_none() {
            return Err(not_found(&format!("reserved Drive folder {} ({})", binding.sync_name, binding.id)));
        }
        Ok(Some(binding.id))
    }

    /// Called only with fresh successful identity evidence or immediately
    /// after a confirmed create. The record is durable before its ID is used.
    pub(super) fn bind_folder_path(
        &self, parent: &str, title: &str, id: &str, segment: &str,
    ) -> io::Result<FolderBinding> {
        let parent = self.actual_parent_id(parent)?;
        let sync_name = match super::duplicates::parse_marker(segment) {
            Some((plain, prefix)) if super::names::decode(plain)? == title => {
                format!("{title}{}{}{}", super::duplicates::MARKER_PREFIX, prefix, super::duplicates::MARKER_SUFFIX)
            }
            _ if representable(title) => title.to_string(),
            _ => super::names::encode(title),
        };
        self.binding_store.transact(&self.drive_account_key, &parent, |record| {
            record.bind_exact(title, id, segment, &sync_name, true)
        })
    }

    pub(super) fn sync_child_locator(&self, parent: &str, name: &str) -> io::Result<String> {
        let segment = match self.resolve(parent) {
            Ok(id) => self.binding_store.read(&self.drive_account_key, &self.actual_parent_id(&id)?)?
                .by_name(name).map(|binding| binding.segment.clone())
                .unwrap_or_else(|| super::names::encode(name)),
            // A new target's intermediate parent may not exist yet. Its new
            // marker-looking names are literal until that parent's registry
            // proves a different origin. Creation still checks reserved IDs.
            Err(error) if error.kind() == io::ErrorKind::NotFound => super::names::encode(name),
            Err(error) => return Err(error),
        };
        Ok(format!("{}/{}", parent.trim_end_matches('/'), segment))
    }

    pub(super) fn project_folders(
        &self, path: &str, raw: &[RawEntry],
    ) -> io::Result<HashMap<String, FolderBinding>> {
        let parent = self.actual_parent_id(&self.resolve(path)?)?;
        let cached = self.valid_folder_hints(path, &parent, raw)?;
        let before = self.binding_store.read(&self.drive_account_key, &parent)?;
        // A complete list alone must not turn a known ID into missing.
        // Verify every absent identity freshly before exposing absence.
        for binding in &before.folders {
            if raw.iter().any(|entry| entry.meta.is_dir
                && entry.meta.id.as_deref() == Some(binding.id.as_str())
                && entry.meta.name == binding.title)
            {
                continue;
            }
            if self.folder_evidence(&parent, &binding.title, &binding.id)?.is_some() {
                return Err(io::Error::new(io::ErrorKind::WouldBlock,
                    "Drive listing omitted an existing bound folder; retry the complete scan"));
            }
        }
        let merged = self.binding_store.transact(&self.drive_account_key, &parent, |current| {
            // Another process may have registered a formerly unknown ID
            // while evidence was gathered. Re-read and validate it before
            // using this snapshot; no network call runs under the lock.
            if current != &before { return Ok(None); }
            super::sync_projection_names::allocate_folders(current, raw, &cached).map(Some)
        })?;
        merged.ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock,
            "Drive folder bindings changed during listing; retry the complete scan"))
    }

    fn valid_folder_hints(
        &self, path: &str, parent: &str, raw: &[RawEntry],
    ) -> io::Result<Vec<FolderBinding>> {
        let mut hints = Vec::new();
        let known = self.binding_store.read(&self.drive_account_key, parent)?;
        let cached = self.ids_guard()?.clone();
        let mimes = self.mimes_guard()?.clone();
        let path = norm(path);
        for (key, id) in cached {
            let (cache_parent, segment) = split_parent(&key);
            if cache_parent != path { continue; }
            // New account-cache snapshots contain projections, not evidence
            // of an old job root's historical selection.
            if self.captured_legacy_id(&key)?.as_deref() != Some(id.as_str()) { continue; }
            if let Some(binding) = known.by_segment(segment) {
                if binding.proves(segment) {
                    continue;
                }
            }
            let entry = raw.iter().find(|entry| entry.meta.is_dir
                && entry.meta.id.as_deref() == Some(id.as_str()));
            let folder_hint = mimes.get(&key).is_some_and(|mime| mime == super::api::FOLDER_MIME);
            if entry.is_none() && !folder_hint { continue; }
            let plain = super::duplicates::parse_marker(segment).map(|(plain, _)| plain).unwrap_or(segment);
            let title = super::names::decode(plain)?;
            if !representable(&title) { continue; }
            let sync_name = match super::duplicates::parse_marker(segment) {
                Some((_, prefix)) if id.starts_with(prefix) => {
                    format!("{title}{}{}{}", super::duplicates::MARKER_PREFIX, prefix, super::duplicates::MARKER_SUFFIX)
                }
                None => title.clone(),
                _ => continue,
            };
            if self.folder_evidence(parent, &title, &id)?.is_none() {
                if folder_hint {
                    return Err(not_found(&format!("previous Drive folder {key} ({id})")));
                }
                continue;
            }
            if !entry.is_some_and(|entry| entry.meta.name == title) {
                return Err(io::Error::new(io::ErrorKind::WouldBlock,
                    "Drive listing omitted a valid previous folder; retry the complete scan"));
            }
            hints.push(FolderBinding { id, title, sync_name,
                segment: segment.to_string(), aliases: Vec::new(), evidence: Some(vec![segment.to_string()]) });
        }
        hints.sort_by(|left, right| left.segment.cmp(&right.segment));
        Ok(hints)
    }

    pub(super) fn sync_meta(&self, path: &str) -> io::Result<crate::vfs::VfsMeta> {
        let key = norm(path);
        if key.is_empty() {
            return crate::vfs::Backend::stat(self, path);
        }
        let (parent_path, segment) = split_parent(&key);
        let parent = self.actual_parent_id(&self.resolve(&parent_path)?)?;
        // Root provenance is enforced by the same walk used by ordinary IO;
        // a direct registry hit must not bypass an unknown old root's guard.
        let resolved = self.resolve(&key)?;
        let binding = self.bound_folder(&parent, segment)?;
        let (id, expected_title) = match &binding {
            Some(binding) => (binding.id.clone(), binding.title.clone()),
            None => {
                let plain = super::duplicates::parse_marker(segment).map(|(plain, _)| plain).unwrap_or(segment);
                let title = super::names::decode(plain)?;
                (resolved, title)
            }
        };
        let json = match self.object_json(&id) {
            Ok(json) => json,
            Err(error) if super::overload::http_status(&error) == Some(404) => return Err(not_found(&key)),
            Err(error) => return Err(error),
        };
        if !super::identity::in_parent(&json, &parent)?
            || super::identity::text(&json, "name")? != expected_title
        {
            return Err(not_found(&key));
        }
        if let Some(problem) = Self::sync_metadata_problem(&json) {
            return Err(invalid(problem));
        }
        let mut meta = Self::meta_from_json(&json, None).ok_or_else(|| invalid("Drive sync metadata has no title"))?;
        if let Some(binding) = binding {
            if !meta.is_dir { return Err(invalid("Drive bound folder changed object type")); }
            meta.name = binding.sync_name;
        } else if meta.is_dir {
            let binding = self.bind_folder_path(&parent, &expected_title, &id, segment)?;
            meta.name = binding.sync_name;
        }
        self.remember_path(&key, &id, json["mimeType"].as_str())?;
        Ok(meta)
    }
}

pub(super) fn representable(name: &str) -> bool {
    !name.is_empty() && !matches!(name, "." | "..") && !name.contains('/') && !name.contains('\0')
}

pub(super) fn child_key(parent: &str, segment: &str) -> String {
    let parent = norm(parent);
    if parent.is_empty() { segment.to_string() } else { format!("{parent}/{segment}") }
}
