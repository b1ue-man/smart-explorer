//! Literal sync names and exact object IDs. Browsing keeps its existing
//! encoded/marker namespace; ambiguous folder subtrees are protected omissions.
use super::GDriveBackend;
use crate::vfs::{OmissionReason, VfsListing, VfsOmission, VfsResult};
use std::collections::{HashMap, HashSet};

impl GDriveBackend {
    pub(super) fn sync_listing(&self, path: &str) -> VfsResult<VfsListing> {
        let raw = self.list_dir_entries(path)?;
        let mut counts = HashMap::new();
        let mut ids = HashSet::new();
        for entry in &raw {
            *counts.entry(entry.meta.name.as_str()).or_insert(0usize) += 1;
            if !ids.insert(entry.meta.id.as_deref()) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
                    "Drive listing repeated an object ID"));
            }
        }
        let mut listing = VfsListing::default();
        for entry in &raw {
            let name = &entry.meta.name;
            let reason = if name.is_empty() || matches!(name.as_str(), "." | "..")
                || name.contains('/') || name.contains('\0') {
                Some((OmissionReason::Unrepresentable, "Drive title is not one sync path component"))
            } else if let Some(problem) = entry.sync_problem {
                Some((OmissionReason::Unreadable, problem))
            } else if entry.meta.is_dir && counts[name.as_str()] > 1 {
                Some((OmissionReason::Unreadable, "Drive folder title has multiple object IDs"))
            } else { None };
            if let Some((reason, detail)) = reason {
                listing.omitted.push(VfsOmission { rel: name.clone(), reason,
                    detail: format!("{detail}: {}", entry.meta.id.as_deref().unwrap_or("")) });
                continue;
            }
            // Only unique names make path hints. Duplicate files are carried
            // with their distinct IDs for the caller's duplicate policy.
            if counts[name.as_str()] == 1 {
                let child = crate::vfs::sync_child_path(self, path, name)?;
                self.remember_path(&child, entry.meta.id.as_deref().unwrap_or(""), entry.mime.as_deref())?;
            }
            listing.entries.push(entry.meta.clone());
        }
        self.persist_path_cache();
        Ok(listing)
    }
}
