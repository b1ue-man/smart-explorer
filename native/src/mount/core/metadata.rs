use super::engine::{
    lock, not_found, parent_path, read_lock, EntryState, MountEngine, OpenHandleKind,
};
use super::types::{Baseline, HandleId};
use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

enum StatOverlay {
    Cached(VfsMeta),
    Remote(String),
}

impl MountEngine {
    pub fn stat(&self, callback_path: &str) -> io::Result<VfsMeta> {
        let path = match self.stat_overlay(callback_path)? {
            StatOverlay::Cached(meta) => return Ok(meta),
            StatOverlay::Remote(path) => path,
        };
        // The remote fetch runs outside the namespace lock: a stalled backend
        // request must wedge only this lookup, never queue a writer that
        // blocks every other callback on the drive.
        self.backend.stat(&path)
    }

    /// Metadata-only lookup for Dokany query callbacks and non-mutating
    /// open-existing admission. Creates, overwrite/delete dispositions, and
    /// every mutation continue to use `stat`.
    pub(crate) fn stat_cached(&self, callback_path: &str) -> io::Result<VfsMeta> {
        let path = match self.stat_overlay(callback_path)? {
            StatOverlay::Cached(meta) => return Ok(meta),
            StatOverlay::Remote(path) => path,
        };
        self.cached_remote_stat(&path)
    }

    /// The lock-holding prefix both stat flavors share: projection plus the
    /// local materialized-entry overlay.
    fn stat_overlay(&self, callback_path: &str) -> io::Result<StatOverlay> {
        let _namespace = read_lock(&self.namespace)?;
        let path = self.project_checked(callback_path)?;
        if let Some(entry) = self.entry_for_path(path.backend())? {
            let state = lock(&entry.state)?;
            if state.delete_token.is_some() {
                return Err(not_found(path.backend()));
            }
            return self.entry_meta(&state).map(StatOverlay::Cached);
        }
        Ok(StatOverlay::Remote(path.backend().to_string()))
    }

    /// Metadata for the object addressed by an already-open file handle. This
    /// deliberately survives a delete-sharing namespace replace, where the
    /// old handle remains valid but its former pathname names a new object.
    pub fn stat_handle(&self, handle: HandleId) -> io::Result<VfsMeta> {
        let _reap = self.operation_reaper();
        match self.handle(handle)?.kind {
            OpenHandleKind::Materialized(entry) => {
                let state = lock(&entry.state)?;
                self.entry_meta(&state)
            }
            OpenHandleKind::Metadata { meta, .. } => Ok(meta),
        }
    }

    pub fn list_dir(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>> {
        self.list_dir_cached(callback_path)
            .map(|entries| entries.to_vec())
    }

    pub(crate) fn list_dir_cached(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>> {
        let (path, depth) = {
            let _namespace = read_lock(&self.namespace)?;
            let path = self.project_checked(callback_path)?;
            let depth = path
                .relative()
                .split('/')
                .filter(|part| !part.is_empty())
                .count()
                .min(u8::MAX as usize) as u8;
            if let Some(listed) = self.metadata_cache.directory(path.backend())? {
                return self.overlay_listing(&path, listed);
            }
            (path, depth)
        };
        // A cold directory needs a remote listing; like whole-file fetches it
        // must run outside the namespace lock so a slow or stalled backend
        // wedges only this folder. The per-directory load slot serializes
        // duplicate fetches and installation revalidates the snapshot.
        let listed = self.cached_remote_directory(path.backend(), depth)?;
        let _namespace = read_lock(&self.namespace)?;
        self.overlay_listing(&path, listed)
    }

    fn overlay_listing(
        &self,
        path: &super::path::ProjectedPath,
        listed: Arc<[VfsMeta]>,
    ) -> io::Result<Arc<[VfsMeta]>> {
        let parent = self.cache_key(path.backend());
        // Namespace read ownership makes these identity keys stable through
        // selection. Never wait for an unrelated file's upload state mutex.
        let entries = lock(&self.entries)?.iter()
            .filter(|(key, _)| parent_path(key) == parent)
            .map(|(_, entry)| Arc::clone(entry)).collect::<Vec<_>>();
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
        self.metadata_cache.validate_listing(&listed)?;
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
