use std::io::{Read, Write};

use super::{
    Backend, BackendHandle, HashHit, Scheme, SearchHit, VfsChangeBatch, VfsMeta, VfsResult,
};

/// Wraps any backend with a short-TTL **directory-listing cache** so interactive
/// browsing (back/forward, re-visiting a folder, rapid drilling) doesn't re-list
/// over the network every time. Mutating ops invalidate the affected directory;
/// `invalidate_cache()` clears everything (explicit refresh). NOT used by sync -
/// sync re-opens a fresh backend per run and walks each folder once, so a cache
/// would only add staleness with no hit benefit.
pub struct CachingBackend {
    inner: BackendHandle,
    ttl: std::time::Duration,
    cache: std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<VfsMeta>)>>,
}

impl CachingBackend {
    pub fn new(inner: BackendHandle) -> Self {
        Self {
            inner,
            ttl: std::time::Duration::from_secs(20),
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
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

    fn invalidate(&self, path: &str) {
        if let Ok(mut c) = self.cache.lock() {
            let key = Self::norm(path);
            c.remove(&key);
            if let Some(parent) = Self::parent_of(&key) {
                c.remove(&parent); // entry added/removed/renamed changes the parent listing
            }
        }
    }
}

impl Backend for CachingBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let key = Self::norm(path);
        if let Ok(c) = self.cache.lock() {
            if let Some((at, v)) = c.get(&key) {
                if at.elapsed() < self.ttl {
                    return Ok(v.clone());
                }
            }
        }
        let v = self.inner.list_dir(path)?;
        if let Ok(mut c) = self.cache.lock() {
            // Bound memory: a full-tree analytics scan can list thousands of
            // dirs; drop everything once the map gets large rather than grow it
            // unbounded (browsing only needs a small working set).
            if c.len() >= 4096 {
                c.clear();
            }
            c.insert(key, (std::time::Instant::now(), v.clone()));
        }
        Ok(v)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
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
        self.inner.open_write(path)
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
        self.invalidate(src);
        self.invalidate(dst);
        r
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
        self.invalidate(path);
        r
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.mkdir_all(path);
        self.invalidate(path);
        r
    }
    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
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
        if let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
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
    ) -> Option<crate::agent_proto::WireNode> {
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
        self.invalidate(root); // new tree appears under root + its parent listing
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
    ) -> bool {
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
    ) -> bool {
        self.inner.walk_hashed(root, want_hash, tx, cancel)
    }
}
