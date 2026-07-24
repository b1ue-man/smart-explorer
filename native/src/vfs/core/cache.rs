use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    Backend, BackendHandle, DeleteDisposition, HashHit, Scheme, SearchHit, VfsChangeBatch, VfsMeta,
    VfsResult,
};

#[path = "cache_index.rs"]
mod cache_index;
#[path = "cache_support.rs"]
mod cache_support;
#[path = "cache_writer.rs"]
mod cache_writer;
use cache_index::{ChildKey, EntryIndex};
use cache_support::*;
use cache_writer::InvalidatingWriter;

/// Wraps any backend with a short-TTL **directory-listing cache** so interactive
/// browsing (back/forward, re-visiting a folder, rapid drilling) doesn't re-list
/// over the network every time. Mutating ops invalidate the affected directory;
/// `invalidate_cache()` clears everything (explicit refresh). NOT used by sync -
/// sync re-opens a fresh backend per run and walks each folder once, so a cache
/// would only add staleness with no hit benefit.
const CACHE_TTL: Duration = Duration::from_secs(20);
// Keep both UI and mounted browsing caches on a fixed metadata budget. A
// single wide directory is admitted only when it fits the global entry/byte
// limits; older directories are evicted by access recency.
const MAX_CACHED_DIRECTORIES: usize = 4_096;
const MAX_CACHED_ENTRIES: usize = 50_000;
const MAX_CACHED_BYTES: usize = 32 * 1024 * 1024;

struct CachedDirectory {
    stored_at: Instant,
    entries: Arc<[VfsMeta]>,
    entry_index: Arc<EntryIndex>,
    entry_count: usize,
    byte_count: usize,
    last_touch: u64,
}

#[derive(Default)]
pub(super) struct CacheState {
    directories: HashMap<String, CachedDirectory>,
    entries: usize,
    bytes: usize,
    clock: u64,
    generation: u64,
}

pub struct CachingBackend {
    inner: BackendHandle,
    cache: Arc<Mutex<CacheState>>,
    child_key: ChildKey,
}

impl CachingBackend {
    pub fn new(inner: BackendHandle) -> Self {
        Self::with_child_key(inner, cache_index::exact_child_key)
    }

    pub(crate) fn with_child_key(inner: BackendHandle, child_key: fn(&str) -> String) -> Self {
        Self {
            inner,
            cache: Arc::new(Mutex::new(CacheState::default())),
            child_key,
        }
    }

    fn norm(path: &str) -> String {
        let p = path.trim_end_matches('/');
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    }

    fn parent_of(key: &str) -> Option<String> {
        key.rfind('/').map(|i| {
            if i == 0 {
                "/".to_string()
            } else {
                key[..i].to_string()
            }
        })
    }

    fn parent_and_name(key: &str) -> Option<(String, &str)> {
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

    fn cached_child_meta(&self, key: &str) -> Option<VfsMeta> {
        let (parent, name) = Self::parent_and_name(key)?;
        let mut cache = self.cache.lock().ok()?;
        let expired = cache
            .directories
            .get(&parent)
            .is_some_and(|cached| cached.stored_at.elapsed() >= CACHE_TTL);
        if expired {
            remove_directory(&mut cache, &parent);
            return None;
        }
        let touch = tick(&mut cache);
        let cached = cache.directories.get_mut(&parent)?;
        cached.last_touch = touch;
        let key = (self.child_key)(name);
        cache_index::lookup(&cached.entries, &cached.entry_index, &key)
            .ok()
            .flatten()
    }

    fn invalidate(&self, path: &str) {
        invalidate_shared(&self.cache, path);
    }

    fn invalidate_prefix(&self, path: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.generation = cache.generation.wrapping_add(1);
            let key = Self::norm(path);
            let child_prefix = if key == "/" {
                "/".to_string()
            } else {
                format!("{key}/")
            };
            let removed = cache
                .directories
                .keys()
                .filter(|cached| *cached == &key || cached.starts_with(&child_prefix))
                .cloned()
                .collect::<Vec<_>>();
            for cached in removed {
                remove_directory(&mut cache, &cached);
            }
            if let Some(parent) = Self::parent_of(&key) {
                remove_directory(&mut cache, &parent);
            }
        }
    }

    fn invalidate_ancestors(&self, path: &str) {
        cache_support::invalidate_ancestors(&self.cache, path);
    }

    fn directory_snapshot(&self, path: &str) -> VfsResult<Arc<[VfsMeta]>> {
        let key = Self::norm(path);
        let generation = match self.cache.lock() {
            Ok(mut cache) => {
                let expired = cache
                    .directories
                    .get(&key)
                    .is_some_and(|cached| cached.stored_at.elapsed() >= CACHE_TTL);
                if expired {
                    remove_directory(&mut cache, &key);
                } else {
                    let touch = tick(&mut cache);
                    if let Some(cached) = cache.directories.get_mut(&key) {
                        cached.last_touch = touch;
                        return Ok(Arc::clone(&cached.entries));
                    }
                }
                cache.generation
            }
            Err(_) => return Ok(Arc::<[VfsMeta]>::from(self.inner.list_dir(path)?)),
        };
        let entries = Arc::<[VfsMeta]>::from(self.inner.list_dir(path)?);
        let entry_count = entries.len().saturating_add(1);
        if entry_count <= MAX_CACHED_ENTRIES {
            let metadata_bytes = cached_metadata_bytes(&key, &entries);
            if metadata_bytes <= MAX_CACHED_BYTES {
                let (entry_index, index_bytes) = cache_index::build(&entries, self.child_key);
                let byte_count = cached_bytes(metadata_bytes, index_bytes);
                if byte_count <= MAX_CACHED_BYTES {
                    let entry_index = Arc::new(entry_index);
                    if let Ok(mut cache) = self.cache.lock() {
                        if cache.generation != generation {
                            return Ok(entries);
                        }
                        purge_expired(&mut cache);
                        remove_directory(&mut cache, &key);
                        evict_until(&mut cache, entry_count, byte_count);
                        if cache.directories.len() < MAX_CACHED_DIRECTORIES
                            && fits(&cache, entry_count, byte_count)
                        {
                            let last_touch = tick(&mut cache);
                            cache.entries += entry_count;
                            cache.bytes += byte_count;
                            cache.directories.insert(
                                key,
                                CachedDirectory {
                                    stored_at: Instant::now(),
                                    entries: Arc::clone(&entries),
                                    entry_index,
                                    entry_count,
                                    byte_count,
                                    last_touch,
                                },
                            );
                        }
                    }
                }
            }
        }
        Ok(entries)
    }

    /// Resolves one child from the retained snapshot without cloning or
    /// rescanning a wide directory on every path component.
    pub(crate) fn unique_child(&self, parent: &str, requested: &str) -> VfsResult<Option<VfsMeta>> {
        let entries = self.directory_snapshot(parent)?;
        let key = Self::norm(parent);
        let requested_key = (self.child_key)(requested);
        if let Ok(mut cache) = self.cache.lock() {
            let touch = tick(&mut cache);
            if let Some(cached) = cache.directories.get_mut(&key) {
                cached.last_touch = touch;
                return cache_index::lookup(&cached.entries, &cached.entry_index, &requested_key);
            }
        }
        cache_index::scan(&entries, requested, self.child_key)
    }
}

impl Backend for CachingBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn state_identity(&self) -> String {
        self.inner.state_identity()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(self.directory_snapshot(path)?.to_vec())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let key = Self::norm(path);
        if let Some(meta) = self.cached_child_meta(&key) {
            return Ok(meta);
        }
        self.inner.stat(path)
    }
    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        // Existence gates mutations, so bypass potentially stale listing data.
        self.inner.try_exists(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }
    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        self.inner.item_id(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read_id(path, id)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        // A new file may appear in the parent listing once written.
        self.invalidate(path);
        let writer = match self.inner.open_write(path) {
            Ok(writer) => writer,
            Err(error) => {
                self.invalidate(path);
                return Err(error);
            }
        };
        Ok(Box::new(InvalidatingWriter::new(
            writer,
            Arc::clone(&self.cache),
            path,
        )))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.invalidate(path);
        let writer = match self.inner.open_write_new(path) {
            Ok(writer) => writer,
            Err(error) => {
                self.invalidate(path);
                return Err(error);
            }
        };
        Ok(Box::new(InvalidatingWriter::new(
            writer,
            Arc::clone(&self.cache),
            path,
        )))
    }
    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let r = self.inner.copy_file(src, dst);
        self.invalidate(dst);
        r
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let r = self.inner.rename(src, dst);
        self.invalidate_prefix(src);
        self.invalidate_prefix(dst);
        r
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        let result = self.inner.rename_no_replace(src, dst);
        self.invalidate_prefix(src);
        self.invalidate_prefix(dst);
        result
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let result = self.inner.promote_staged(staged, destination);
        self.invalidate(staged);
        self.invalidate(destination);
        result
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let result = self.inner.promote_staged_no_replace(staged, destination);
        self.invalidate(staged);
        self.invalidate(destination);
        result
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.remove_file(path);
        self.invalidate(path);
        r
    }
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        let r = self.inner.remove_file_id(path, id);
        self.invalidate(path);
        r
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.remove_dir(path);
        self.invalidate_prefix(path);
        r
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.mkdir_all(path);
        self.invalidate_ancestors(path);
        r
    }
    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn staged_write_capabilities(&self, root: &str) -> super::StagedWriteCapabilities {
        self.inner.staged_write_capabilities(root)
    }
    fn case_sensitive_paths(&self, root: &str) -> bool {
        self.inner.case_sensitive_paths(root)
    }
    fn root_confinement(&self, root: &str) -> super::RootConfinement {
        self.inner.root_confinement(root)
    }
    fn mount_path_capabilities(&self, root: &str) -> VfsResult<super::MountPathCapabilities> {
        self.inner.mount_path_capabilities(root)
    }
    fn plan_dedupe_recursive(
        &self,
        root: &str,
        keep: &dyn Fn(&str) -> bool,
    ) -> VfsResult<Vec<super::DedupeCandidate>> {
        self.inner.plan_dedupe_recursive(root, keep)
    }
    fn apply_dedupe_plan(&self, plan: &[super::DedupeCandidate]) -> VfsResult<usize> {
        let result = self.inner.apply_dedupe_plan(plan);
        self.invalidate_cache(); // an exact plan can span many folders
        result
    }
    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        let r = self.inner.dedupe_recursive(root, keep);
        self.invalidate_cache(); // a recursive change can touch many folders
        r
    }
    fn is_local(&self) -> bool {
        self.inner.is_local()
    }
    fn provides_content_hash(&self) -> bool {
        self.inner.provides_content_hash()
    }
    fn supports_changes(&self) -> bool {
        self.inner.supports_changes()
    }
    fn change_root_id(&self, root: &str) -> VfsResult<Option<String>> {
        self.inner.change_root_id(root)
    }
    fn current_change_cursor(&self, root: &str) -> VfsResult<Option<String>> {
        self.inner.current_change_cursor(root)
    }
    fn changes_since(&self, root: &str, cursor: &str) -> VfsResult<VfsChangeBatch> {
        self.inner.changes_since(root, cursor)
    }
    fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            let generation = cache.generation.wrapping_add(1);
            *cache = CacheState {
                generation,
                ..CacheState::default()
            };
        }
    }
    fn delete_disposition(&self) -> DeleteDisposition {
        self.inner.delete_disposition()
    }
    // Forward the agent capability so analytics' one-shot server-side walk works
    // through the cache wrapper (otherwise it fell back to per-dir listing).
    fn supports_walk_tree(&self) -> bool {
        self.inner.supports_walk_tree()
    }
    fn walk_tree(
        &self,
        root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        self.inner.walk_tree(root, on_progress)
    }
    fn supports_bulk_tree(&self) -> bool {
        self.inner.supports_bulk_tree()
    }
    fn get_tree(&self, root: &str, dst: &std::path::Path) -> VfsResult<u64> {
        self.inner.get_tree(root, dst)
    }
    fn put_tree(&self, src: &std::path::Path, root: &str) -> VfsResult<u64> {
        let r = self.inner.put_tree(src, root);
        self.invalidate_prefix(root);
        r
    }
    fn supports_search(&self) -> bool {
        self.inner.supports_search()
    }
    fn search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: crossbeam_channel::Sender<SearchHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        self.inner.search(root, spec, tx, cancel)
    }
    fn supports_walk_hashed(&self) -> bool {
        self.inner.supports_walk_hashed()
    }
    fn walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: crossbeam_channel::Sender<HashHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        self.inner.walk_hashed(root, want_hash, tx, cancel)
    }
}
