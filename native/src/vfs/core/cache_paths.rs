//! Path keys of the listing cache (normalized directory keys and parents),
//! child lookups in retained snapshots and invalidation after mutations.
use super::cache_index;
use super::cache_retirement::Retirement;
use super::cache_support::{self, cached_snapshot, invalidate_shared};
use super::cache_writer::InvalidatingWriter;
use super::{CachingBackend, VfsMeta, VfsResult};
use std::io::Write;
use std::sync::Arc;

impl CachingBackend {
    pub(super) fn norm(path: &str) -> String {
        let p = path.trim_end_matches('/');
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    }

    pub(super) fn parent_of(key: &str) -> Option<String> {
        if key == "/" {
            return None;
        }
        key.rfind('/').map(|i| {
            if i == 0 {
                "/".to_string()
            } else {
                key[..i].to_string()
            }
        })
    }

    pub(super) fn parent_and_name(key: &str) -> Option<(String, &str)> {
        if key.is_empty() || key == "/" {
            return None;
        }
        match key.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => Some((
                if parent.is_empty() {
                    "/".to_string()
                } else {
                    parent.to_string()
                },
                name,
            )),
            None => Some(("/".to_string(), key)),
            _ => None,
        }
    }

    pub(super) fn cached_child_meta(&self, key: &str) -> Option<VfsMeta> {
        let (parent, name) = Self::parent_and_name(key)?;
        let mut retired = Retirement::default();
        let snapshot = {
            let mut cache = self.cache.lock().ok()?;
            cached_snapshot(&mut cache, &parent, &mut retired)?
        };
        drop(retired);
        let key = (self.child_key)(name);
        cache_index::lookup(&snapshot.entries, &snapshot.index, &key)
            .ok()
            .flatten()
    }

    /// A writer of the inner backend for `path`, with the listing
    /// invalidated before and after opening and again when it commits.
    pub(super) fn invalidating_writer(
        &self,
        path: &str,
        open: impl FnOnce() -> VfsResult<Box<dyn Write + Send>>,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.invalidate(path);
        let result = open();
        self.invalidate(path);
        result.map(|writer| self.wrap_writer(writer, path))
    }

    pub(super) fn wrap_writer(
        &self,
        writer: Box<dyn Write + Send>,
        path: &str,
    ) -> Box<dyn Write + Send> {
        Box::new(InvalidatingWriter::new(
            writer,
            Arc::clone(&self.cache),
            path,
        ))
    }

    pub(super) fn invalidate(&self, path: &str) {
        invalidate_shared(&self.cache, path);
    }

    pub(super) fn invalidate_prefix(&self, path: &str) {
        cache_support::invalidate_prefix(&self.cache, path);
    }

    pub(super) fn invalidate_ancestors(&self, path: &str) {
        cache_support::invalidate_ancestors(&self.cache, path);
    }

    /// Resolves one child from the retained snapshot without cloning or
    /// rescanning a wide directory on every path component.
    pub(crate) fn unique_child(&self, parent: &str, requested: &str) -> VfsResult<Option<VfsMeta>> {
        let snapshot = self.directory_snapshot(parent)?;
        let requested_key = (self.child_key)(requested);
        cache_index::lookup(&snapshot.entries, &snapshot.index, &requested_key)
    }
}
