use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink,
    DeleteDisposition, HashHit, Scheme, SearchHit, VfsChangeBatch, VfsMeta, VfsResult,
};

#[path = "cache_index.rs"]
mod cache_index;
#[path = "cache_load.rs"]
mod cache_load;
#[path = "cache_paths.rs"]
mod cache_paths;
#[path = "cache_retirement.rs"]
mod cache_retirement;
#[path = "cache_support.rs"]
mod cache_support;
#[path = "cache_writer.rs"]
mod cache_writer;
use cache_index::ChildKey;
use cache_load::{DirectoryLoad, DirectorySnapshot};
use cache_writer::InvalidatingWriter;

#[cfg(test)]
#[path = "vault_cache_task_tests.rs"]
mod vault_cache_task_tests;

/// Wraps any backend with a short-TTL **directory-listing cache** so interactive
/// browsing (back/forward, re-visiting a folder, rapid drilling) doesn't re-list
/// over the network every time. Mutating ops invalidate the affected directory;
/// `invalidate_cache()` clears everything (explicit refresh). NOT used by sync -
/// sync re-opens a fresh backend per run and walks each folder once, so a cache
/// would only add staleness with no hit benefit.
const CACHE_TTL: Duration = Duration::from_secs(20);
#[derive(Clone, Copy)]
struct CacheLimits {
    directories: usize,
    entries: usize,
    bytes: usize,
}

impl CacheLimits {
    const BROWSING: Self = Self {
        directories: 4_096,
        entries: 50_000,
        bytes: 32 * 1024 * 1024,
    };
    // Mount limits govern retention, never directory validity or traversal.
    const MOUNT: Self = Self {
        directories: usize::MAX,
        entries: usize::MAX,
        bytes: 64 * 1024 * 1024,
    };
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
    loads: BTreeMap<String, Weak<DirectoryLoad>>,
    entries: usize,
    bytes: usize,
    clock: u64,
    // Only explicit whole-cache invalidation advances this epoch. Ordinary
    // mutations fence the affected live DirectoryLoad revisions instead.
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
        let mut cache =
            Self::with_child_key(inner, child_key.unwrap_or(cache_index::exact_child_key));
        cache.limits = CacheLimits::MOUNT;
        cache
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
    fn namespace_identity(&self) -> String {
        self.inner.namespace_identity()
    }
    fn uncached_backend(&self) -> Option<BackendHandle> {
        Some(
            self.inner
                .uncached_backend()
                .unwrap_or_else(|| self.inner.clone()),
        )
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
    fn read_size(&self, path: &str, metadata_size: u64) -> VfsResult<Option<u64>> {
        self.inner.read_size(path, metadata_size)
    }
    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.invalidating_writer(path, || self.inner.open_write_copy_stage(path))
    }
    fn promote_copy_stage(&self, staged: &str, destination: &str) -> VfsResult<()> {
        let result = self.inner.promote_copy_stage(staged, destination);
        self.invalidate(staged);
        self.invalidate(destination);
        result
    }
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.invalidating_writer(path, || self.inner.open_write_copy_stage_sized(path, size))
    }
    fn open_write_copy_stage_unsynced(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.invalidating_writer(path, || {
            self.inner.open_write_copy_stage_unsynced(path, size)
        })
    }
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        let result = self.inner.server_copy_to_stage(src, stage, size, cancel);
        self.invalidate(stage);
        result
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
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        let result = self.inner.create_dir(path);
        self.invalidate_ancestors(path);
        result
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        let result = self.inner.create_dir_new(path);
        self.invalidate_ancestors(path);
        result
    }
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        let result = self.inner.discard_copy_stage(stage);
        self.invalidate(stage);
        result
    }
    fn open_write_fresh(&self, path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>> {
        self.invalidate(path);
        let writer = self.inner.open_write_fresh(path, size)?;
        Ok(writer.map(|writer| self.wrap_writer(writer, path)))
    }
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        self.inner.open_read_at(path, id, offset)
    }
    fn transfer_hint(&self) -> Option<String> {
        self.inner.transfer_hint()
    }
    fn flow_key(&self, path: &str) -> String {
        self.inner.flow_key(path)
    }
    fn transfer_ceiling(&self, path: &str) -> Option<usize> {
        self.inner.transfer_ceiling(path)
    }
    fn concurrent_read_write(&self) -> bool {
        self.inner.concurrent_read_write()
    }
    fn batch_limits(&self, dir: &str) -> Option<BatchLimits> {
        self.inner.batch_limits(dir)
    }
    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        let result = self.inner.put_batch(entries, data);
        for entry in entries {
            self.invalidate(&entry.path);
        }
        if let Ok(outcomes) = &result {
            for outcome in outcomes {
                if let BatchPutOutcome::Published(path) = outcome {
                    self.invalidate(path);
                }
            }
        }
        result
    }
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        self.inner.get_batch(items, sink)
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
        let retired = if let Ok(mut cache) = self.cache.lock() {
            let generation = cache.generation.wrapping_add(1);
            let loads = std::mem::take(&mut cache.loads);
            Some(std::mem::replace(
                &mut *cache,
                CacheState {
                    generation,
                    loads,
                    ..CacheState::default()
                },
            ))
        } else {
            None
        };
        // The global epoch fences every live flight; registrations survive so
        // their waiters still serialize. Old snapshot destructors run unlocked.
        drop(retired);
    }
    fn delete_disposition(&self) -> DeleteDisposition {
        self.inner.delete_disposition()
    }
    // Forward the agent capability so analytics' one-shot server-side walk works
    // through the cache wrapper (otherwise it fell back to per-dir listing).
    fn scan_storage(
        &self,
        root: &str,
        progress: &crate::analytics::Progress,
    ) -> VfsResult<Option<crate::analytics::ScanOutcome>> {
        self.inner.scan_storage(root, progress)
    }
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
