//! Literal sync names and exact object IDs. Browsing keeps its existing
//! encoded/marker namespace; independent folders have stable logical aliases.
use super::GDriveBackend;
use crate::vfs::{OmissionReason, VfsListing, VfsOmission, VfsResult};
use std::collections::HashMap;

impl GDriveBackend {
    pub(super) fn sync_listing(&self, path: &str) -> VfsResult<VfsListing> {
        let (raw, folders) = self.projected_listing(path)?;
        let parent = self.resolve(path)?;
        let record = self.binding_store.read(&self.drive_account_key, &parent)?;
        let mut file_counts = HashMap::new();
        for entry in raw.iter().filter(|entry| !entry.meta.is_dir) {
            *file_counts.entry(entry.meta.name.as_str()).or_insert(0usize) += 1;
        }
        let mut listing = VfsListing::default();
        for entry in &raw {
            let projection = entry.meta.id.as_deref().and_then(|id| folders.get(id));
            let name = projection.map(|folder| &folder.sync_name).unwrap_or(&entry.meta.name);
            let reason = if !super::sync_projection::representable(&entry.meta.name) {
                Some((
                    OmissionReason::Unrepresentable,
                    "Drive title is not one sync path component",
                ))
            } else if let Some(problem) = entry.sync_problem {
                Some((OmissionReason::Unreadable, problem))
            } else if !entry.meta.is_dir && record.by_name(name).is_some() {
                Some((
                    OmissionReason::Unreadable,
                    "Drive literal file title collides with a reserved folder alias",
                ))
            } else {
                None
            };
            if let Some((reason, detail)) = reason {
                listing.omitted.push(VfsOmission {
                    rel: name.clone(),
                    reason,
                    detail: format!("{detail}: {}", entry.meta.id.as_deref().unwrap_or("")),
                });
                continue;
            }
            // Folder locators are exact registered IDs. Files retain literal
            // locators and captured IDs for the existing duplicate policy.
            if entry.meta.is_dir || file_counts[name.as_str()] == 1 {
                let segment = projection.map(|folder| folder.segment.clone())
                    .unwrap_or_else(|| super::names::encode(name));
                let child = super::sync_projection::child_key(path, &segment);
                self.remember_path(
                    &child,
                    entry.meta.id.as_deref().unwrap_or(""),
                    entry.mime.as_deref(),
                )?;
            }
            let mut meta = entry.meta.clone();
            meta.name = name.clone();
            listing.entries.push(meta);
        }
        self.persist_path_cache();
        Ok(listing)
    }
}
