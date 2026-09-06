use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, Weak};
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
#[path = "cache_load.rs"]
mod cache_load;
use cache_index::ChildKey;
use cache_load::{DirectoryLoad, DirectorySnapshot};
use cache_support::*;
use cache_writer::InvalidatingWriter;

/// Wraps any backend with a short-TTL **directory-listing cache** so interactive
/// browsing (back/forward, re-visiting a folder, rapid drilling) doesn't re-list
/// over the network every time. Mutating ops invalidate the affected directory;
/// `invalidate_cache()` clears everything (explicit refresh). NOT used by sync -
/// sync re-opens a fresh backend per run and walks each folder once, so a cache
/// would only add staleness with no hit benefit.
const CACHE_TTL: Duration = Duration::from_secs(20);
#[derive(Clone, Copy)]
struct CacheLimits { directories: usize, entries: usize, bytes: usize }

impl CacheLimits {
    const BROWSING: Self = Self { directories: 4_096, entries: 50_000, bytes: 32 * 1024 * 1024 };
    // Mount limits govern retention, never directory validity or traversal.
    const MOUNT: Self = Self { directories: usize::MAX, entries: usize::MAX, bytes: 64 * 1024 * 1024 };
}

struct CachedDirectory {
    snapshot: DirectorySnapshot,
    entry_count: usize,
    byte_count: usize,
    last_touch: u64,
}

#[derive(Default)]
pub(super) struct CacheState {
    directories: BTreeMap<String, CachedDirectory>,
    recency: BTreeSet<(u64, String)>,
    expiry: BTreeSet<(Instant, String)>,
    loads: HashMap<String, Weak<DirectoryLoad>>,
    entries: usize,
    bytes: usize,
    clock: u64,
    generation: u64,
}

pub struct CachingBackend {
    inner: BackendHandle,
    cache: Arc<Mutex<CacheState>>,
    child_key: ChildKey,
    limits: CacheLimits,
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
            limits: CacheLimits::BROWSING,
        }
    }

    pub(crate) fn for_mount(inner: BackendHandle, child_key: Option<fn(&str) -> String>) -> Self {
        let mut cache = Self::with_child_key(inner, child_key.unwrap_or(cache_index::exact_child_key));
        cache.limits = CacheLimits::MOUNT;
        cache
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
        let snapshot = cached_snapshot(&mut cache, &parent)?;
        let key = (self.child_key)(name);
        cache_index::lookup(&snapshot.entries, &snapshot.index, &key)
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
            let removed = cache.directories.range(child_prefix.clone()..)
                .take_while(|(cached, _)| cached.starts_with(&child_prefix))
                .map(|(cached, _)| cached.clone())
                .collect::<Vec<_>>();
            remove_directory(&mut cache, &key);
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

    /// Resolves one child from the retained snapshot without cloning or
    /// rescanning a wide directory on every path component.
    pub(crate) fn unique_child(&self, parent: &str, requested: &str) -> VfsResult<Option<VfsMeta>> {
        let snapshot = self.directory_snapshot(parent)?;
        let requested_key = (self.child_key)(requested);
        cache_index::lookup(&snapshot.entries, &snapshot.index, &requested_key)
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
        Ok(self.directory_snapshot(path)?.entries.to_vec())
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
            let loads = std::mem::take(&mut cache.loads);
            *cache = CacheState {
                generation,
                loads,
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
