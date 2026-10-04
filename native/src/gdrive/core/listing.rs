//! Raw Drive metadata and browser stat. Complete pagination lives separately
//! from the stable sync projection and from the browser's rendered names.
use super::api::not_found;
use super::core::{norm, split_parent};
use super::identity::{in_parent, invalid};
use super::sync_bindings::FolderBinding;
use super::GDriveBackend;
use crate::vfs::{VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io;

impl GDriveBackend {
    /// Identity evidence may expose a gap even after token-complete paging.
    /// Discard that entire snapshot and collect again under the same bounded
    /// read/backoff contract; neither browsing nor sync receives partial data.
    pub(super) fn projected_listing(
        &self,
        path: &str,
    ) -> VfsResult<(Vec<RawEntry>, HashMap<String, FolderBinding>)> {
        let mut backoff = super::api::listing_backoff();
        let mut attempt = 0;
        loop {
            attempt += 1;
            let raw = self.list_dir_entries(path)?;
            match self.project_folders(path, &raw) {
                Ok(folders) => return Ok((raw, folders)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !super::api::restart_listing_read(attempt, &mut backoff, &error) {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(super) fn list_dir_entries(&self, path: &str) -> VfsResult<Vec<RawEntry>> {
        let parent = self.resolve(path)?;
        self.collect_files(&parent, None)?.into_iter().map(|file| {
            let meta = Self::meta_from_json(&file, None)
                .ok_or_else(|| invalid("Drive listing contains an object without a name"))?;
            Ok(RawEntry {
                meta,
                mime: file["mimeType"].as_str().map(str::to_string),
                sync_problem: Self::sync_metadata_problem(&file),
            })
        }).collect()
    }

    pub(super) fn stat_marker_aware(&self, path: &str) -> VfsResult<VfsMeta> {
        let key = norm(path);
        if key.is_empty() {
            return Ok(VfsMeta { name: "/".into(), is_dir: true, ..VfsMeta::default() });
        }
        let id = self.resolve(&key)?;
        let (parent_path, segment) = split_parent(&key);
        let parent = self.resolve(&parent_path)?;
        let binding = self.bound_folder(&parent, segment)?;
        let title = match binding {
            Some(binding) => binding.title,
            None => {
                let plain = super::duplicates::parse_marker(segment).map(|(plain, _)| plain).unwrap_or(segment);
                super::names::decode(plain)?
            }
        };
        let json = match self.object_json(&id) {
            Ok(json) => json,
            Err(error) if super::overload::http_status(&error) == Some(404) => return Err(not_found(&key)),
            Err(error) => return Err(error),
        };
        if !in_parent(&json, &parent)? || json["name"].as_str() != Some(title.as_str()) {
            return Err(not_found(&key));
        }
        let mut meta = Self::meta_from_json(&json, None).ok_or_else(|| invalid("Drive metadata has no name"))?;
        meta.name = segment.to_string();
        Ok(meta)
    }
}

pub(super) struct RawEntry {
    pub(super) meta: VfsMeta,
    pub(super) mime: Option<String>,
    pub(super) sync_problem: Option<&'static str>,
}
