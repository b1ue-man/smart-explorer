//! Indexed synthetic provider: fixture cost is proportional to one directory,
//! not the size of the complete generated vault. Compiled only into tests.
use crate::mount::optimization_fixture::OptimizationBackend;
use crate::vfs::{Backend, RootConfinement, Scheme, StagedWriteCapabilities, VfsMeta};
use std::{collections::BTreeMap, io::{self, Cursor, Read, Write}, sync::{Arc,
    atomic::{AtomicUsize, Ordering}}, time::Duration};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VaultTaskCounters {
    pub lists: usize,
    pub stats: usize,
    pub reads: usize,
    pub max_active: usize,
}

#[derive(Default)]
struct Counters {
    lists: AtomicUsize, stats: AtomicUsize, reads: AtomicUsize,
    active: AtomicUsize, max_active: AtomicUsize,
}

pub(super) struct VaultTree {
    pub source: Arc<OptimizationBackend>,
    nodes: BTreeMap<String, VfsMeta>,
    children: BTreeMap<String, Vec<VfsMeta>>,
    counters: Counters,
}

impl VaultTree {
    pub fn new() -> Arc<Self> {
        let mut tree = Self { source: OptimizationBackend::new(), nodes: BTreeMap::new(),
            children: BTreeMap::new(), counters: Counters::default() };
        tree.add("/large", true);
        tree.add("/wide", true);
        for branch in 0..512 {
            let mut path = format!("/large/b{branch:03}");
            tree.add(&path, true);
            for depth in 0..8 {
                path.push_str(&format!("/d{depth}"));
                tree.add(&path, true);
                for note in 0..4 { tree.add(&format!("{path}/note{note}.md"), false); }
            }
        }
        for file in 0..50_001 { tree.add(&format!("/wide/f{file:05}.md"), false); }
        Arc::new(tree)
    }

    fn add(&mut self, path: &str, directory: bool) {
        let (parent, name) = path.rsplit_once('/').expect("fixture absolute child");
        let metadata = VfsMeta { name: name.into(), is_dir: directory,
            size: if directory { 0 } else { 4 }, mtime_ms: 1_700_000_000_000,
            id: Some(format!("vault:{path}")), ..VfsMeta::default() };
        self.children.entry(if parent.is_empty() { "/" } else { parent }.into())
            .or_default().push(metadata.clone());
        if directory { self.children.entry(path.into()).or_default(); }
        self.nodes.insert(path.into(), metadata);
    }

    pub fn counters(&self) -> VaultTaskCounters {
        VaultTaskCounters { lists: self.counters.lists.load(Ordering::SeqCst),
            stats: self.counters.stats.load(Ordering::SeqCst),
            reads: self.counters.reads.load(Ordering::SeqCst),
            max_active: self.counters.max_active.load(Ordering::SeqCst) }
    }

    fn mutable(&self, paths: &[&str]) -> io::Result<()> {
        if paths.iter().any(|path| *path == "/large" || path.starts_with("/large/")
            || *path == "/wide" || path.starts_with("/wide/"))
        { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "immutable metadata fixture")); }
        Ok(())
    }
}

struct Active<'a>(&'a Counters);
impl Drop for Active<'_> {
    fn drop(&mut self) { self.0.active.fetch_sub(1, Ordering::SeqCst); }
}

impl Backend for VaultTree {
    fn scheme(&self) -> Scheme { Scheme::Peer }
    fn root_display(&self) -> String { "/".into() }
    fn parallelism(&self) -> usize { 8 }
    fn case_sensitive_paths(&self, _: &str) -> bool { false }
    fn root_confinement(&self, _: &str) -> RootConfinement { RootConfinement::Enforced }
    fn rename_overwrites(&self) -> bool { true }
    fn staged_write_capabilities(&self, _: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities::complete()
    }

    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        self.counters.stats.fetch_add(1, Ordering::SeqCst);
        self.nodes.get(path).cloned().map(Ok).unwrap_or_else(|| self.source.stat(path))
    }

    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        self.counters.lists.fetch_add(1, Ordering::SeqCst);
        let active = self.counters.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _active = Active(&self.counters);
        self.counters.max_active.fetch_max(active, Ordering::SeqCst);
        // A documented fixed synthetic 1-ms provider latency makes independent
        // request overlap observable. This is not a user's-network benchmark.
        std::thread::sleep(Duration::from_millis(1));
        let mut listed = self.children.get(path).cloned().unwrap_or_default();
        if path == "/" || !self.children.contains_key(path) {
            listed.extend(self.source.list_dir(path)?);
        }
        Ok(listed)
    }

    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        self.counters.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(metadata) = self.nodes.get(path) {
            if metadata.is_dir { return Err(io::Error::other("cannot read directory")); }
            return Ok(Box::new(Cursor::new(b"note".to_vec())));
        }
        self.source.open_read(path)
    }
    fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.mutable(&[path])?; self.source.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.mutable(&[path])?; self.source.open_write_new(path)
    }
    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        self.mutable(&[from, to])?; self.source.rename(from, to)
    }
    fn rename_no_replace(&self, from: &str, to: &str) -> io::Result<()> {
        self.mutable(&[from, to])?; self.source.rename_no_replace(from, to)
    }
    fn promote_staged(&self, from: &str, to: &str) -> io::Result<()> { self.rename(from, to) }
    fn promote_staged_no_replace(&self, from: &str, to: &str) -> io::Result<()> {
        self.rename_no_replace(from, to)
    }
    fn remove_file(&self, path: &str) -> io::Result<()> {
        self.mutable(&[path])?; self.source.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> io::Result<()> {
        self.mutable(&[path])?; self.source.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> io::Result<()> {
        self.mutable(&[path])?; self.source.mkdir_all(path)
    }
}
