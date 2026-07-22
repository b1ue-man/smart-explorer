use super::engine::{lock, not_found, parent_path, read_lock, EntryState, MountEngine};
use super::types::{Baseline, HandleId};
use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

impl MountEngine {
    pub fn stat(&self, callback_path: &str) -> io::Result<VfsMeta> {
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        if let Some(entry) = self.entry_for_path(path.backend())? {
            let state = lock(&entry.state)?;
            if state.delete_token.is_some() {
                return Err(not_found(path.backend()));
            }
            return self.entry_meta(&state);
        }
        self.backend.stat(path.backend())
    }

    /// Metadata-only lookup for Dokany query callbacks. Namespace admission,
    /// create/open dispositions, and every mutation continue to use `stat`.
    pub(crate) fn stat_cached(&self, callback_path: &str) -> io::Result<VfsMeta> {
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        if let Some(entry) = self.entry_for_path(path.backend())? {
            let state = lock(&entry.state)?;
            if state.delete_token.is_some() {
                return Err(not_found(path.backend()));
            }
            return self.entry_meta(&state);
        }
        self.cached_remote_stat(path.backend())
    }

    /// Metadata for the object addressed by an already-open file handle. This
    /// deliberately survives a delete-sharing namespace replace, where the
    /// old handle remains valid but its former pathname names a new object.
    pub fn stat_handle(&self, handle: HandleId) -> io::Result<VfsMeta> {
        let entry = self.handle(handle)?.entry;
        let state = lock(&entry.state)?;
        self.entry_meta(&state)
    }

    pub fn list_dir(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>> {
        self.list_dir_cached(callback_path)
            .map(|entries| entries.to_vec())
    }

    pub(crate) fn list_dir_cached(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>> {
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        let depth = path
            .relative()
            .split('/')
            .filter(|part| !part.is_empty())
            .count()
            .min(u8::MAX as usize) as u8;
        let listed = self.cached_remote_directory(path.backend(), depth)?;
        let entries = lock(&self.entries)?.values().cloned().collect::<Vec<_>>();
        let mut overlays = Vec::new();
        for entry in entries {
            let state = lock(&entry.state)?;
            if state.delete_token.is_some()
                || !self.paths_equal(parent_path(&state.remote_path), path.backend())
            {
                continue;
            }
            overlays.push(self.entry_meta(&state)?);
        }
        if overlays.is_empty() {
            return Ok(listed);
        }
        let mut listed = listed.to_vec();
        let mut names = HashMap::<String, usize>::new();
        for (index, meta) in listed.iter().enumerate() {
            names.insert(self.name_key(&meta.name), index);
        }
        for meta in overlays {
            let identity = self.name_key(&meta.name);
            if let Some(index) = names.get(&identity).copied() {
                listed[index] = meta;
            } else {
                names.insert(identity, listed.len());
                listed.push(meta);
            }
        }
        Ok(listed.into())
    }

    fn entry_meta(&self, state: &EntryState) -> io::Result<VfsMeta> {
        let size = self
            .spool
            .open_file(&state.spool_name, false)?
            .metadata()?
            .len();
        let mtime_ms = match &state.baseline {
            Baseline::Missing => 0,
            Baseline::Present { mtime_ms, .. } => *mtime_ms,
        };
        Ok(VfsMeta {
            name: state
                .remote_path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string(),
            size,
            mtime_ms,
            // The spool name follows the materialized object across namespace
            // renames, so Windows observes a stable file index for open handles.
            // Provider identifiers are intentionally not exposed across the
            // rooted backend boundary.
            id: Some(format!("mount-cache:{}", state.spool_name)),
            ..VfsMeta::default()
        })
    }
}
