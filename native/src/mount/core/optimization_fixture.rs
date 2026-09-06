//! Deterministic mutable remote used by the single optimization task suite.
use crate::vfs::{Backend, RootConfinement, Scheme, StagedWriteCapabilities, VfsMeta};
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

struct Node {
    id: u64,
    modified: i64,
    directory: bool,
    link: bool,
    bytes: Vec<u8>,
}

struct StatGate {
    path: String,
    skip: usize,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

struct Inner {
    nodes: Mutex<BTreeMap<String, Arc<Mutex<Node>>>>,
    clock: AtomicI64,
    next_id: AtomicU64,
    reads: AtomicUsize,
    streamed_bytes: AtomicUsize,
    stats: AtomicUsize,
    read_override: Mutex<Option<Vec<u8>>>,
    stat_gate: Mutex<Option<StatGate>>,
    fail_upload: AtomicBool,
}

pub(crate) struct OptimizationBackend(Arc<Inner>);

impl OptimizationBackend {
    pub(crate) fn new() -> Arc<Self> {
        let backend = Arc::new(Self(Arc::new(Inner {
            nodes: Mutex::new(BTreeMap::new()),
            clock: AtomicI64::new(1_700_000_000_000),
            next_id: AtomicU64::new(1),
            reads: AtomicUsize::new(0),
            streamed_bytes: AtomicUsize::new(0),
            stats: AtomicUsize::new(0),
            read_override: Mutex::new(None),
            stat_gate: Mutex::new(None),
            fail_upload: AtomicBool::new(false),
        })));
        backend.mkdir("/");
        backend
    }

    fn node(&self, directory: bool) -> Arc<Mutex<Node>> {
        Arc::new(Mutex::new(Node {
            id: self.0.next_id.fetch_add(1, Ordering::Relaxed),
            modified: self.tick(),
            directory,
            link: false,
            bytes: Vec::new(),
        }))
    }

    fn tick(&self) -> i64 {
        self.0.clock.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn put(&self, path: &str, bytes: &[u8]) {
        self.mkdir(parent(path));
        let mut writer = self.open_write(path).expect("fixture put open");
        writer.write_all(bytes).expect("fixture put write");
        writer.flush().expect("fixture put flush");
    }

    pub(crate) fn mkdir(&self, path: &str) {
        self.mkdir_all(path).expect("fixture mkdir");
    }

    pub(crate) fn bytes(&self, path: &str) -> Vec<u8> {
        let node = self.lookup(path).expect("fixture file exists");
        let node = node.lock().unwrap();
        assert!(!node.directory && !node.link);
        node.bytes.clone()
    }

    pub(crate) fn read_count(&self) -> usize {
        self.0.reads.load(Ordering::SeqCst)
    }

    pub(crate) fn stat_count(&self) -> usize {
        self.0.stats.load(Ordering::SeqCst)
    }

    pub(crate) fn read_byte_count(&self) -> usize {
        self.0.streamed_bytes.load(Ordering::SeqCst)
    }

    pub(crate) fn override_next_read(&self, bytes: Vec<u8>) {
        *self.0.read_override.lock().unwrap() = Some(bytes);
    }

    pub(crate) fn fail_upload(&self, fail: bool) {
        self.0.fail_upload.store(fail, Ordering::SeqCst);
    }

    pub(crate) fn make_link(&self, path: &str) {
        self.put(path, b"never follow this synthetic link");
        self.lookup(path).unwrap().lock().unwrap().link = true;
    }

    /// Capture a stat result, then pause before returning it. No backend lock
    /// remains held, so a concurrent rename can invalidate the captured result.
    pub(crate) fn gate_stat(&self, path: &str, skip: usize) -> FixtureGate {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let gate = StatGate { path: path.into(), skip, entered: entered_tx, release: release_rx };
        assert!(self.0.stat_gate.lock().unwrap().replace(gate).is_none());
        FixtureGate { entered, release: Some(release) }
    }

    fn lookup(&self, path: &str) -> io::Result<Arc<Mutex<Node>>> {
        validate(path)?;
        self.0.nodes.lock().unwrap().get(path).cloned().ok_or_else(missing)
    }

    fn writer(&self, path: &str, exclusive: bool) -> io::Result<Box<dyn Write + Send>> {
        validate(path)?;
        let mut nodes = self.0.nodes.lock().unwrap();
        require_parent(&nodes, path)?;
        let node = if let Some(node) = nodes.get(path) {
            if exclusive {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "fixture target exists"));
            }
            let mut state = node.lock().unwrap();
            if state.directory || state.link { return Err(invalid()); }
            state.bytes.clear();
            state.modified = self.tick();
            Arc::clone(node)
        } else {
            let node = self.node(false);
            nodes.insert(path.into(), Arc::clone(&node));
            node
        };
        Ok(Box::new(FixtureWriter { inner: Arc::clone(&self.0), node }))
    }

    fn move_node(&self, source: &str, destination: &str, replace: bool) -> io::Result<()> {
        validate(source)?;
        validate(destination)?;
        if source == "/" || destination.starts_with(&format!("{source}/")) { return Err(invalid()); }
        let mut nodes = self.0.nodes.lock().unwrap();
        let source_node = nodes.get(source).cloned().ok_or_else(missing)?;
        require_parent(&nodes, destination)?;
        if source == destination { return Ok(()); }
        if let Some(target) = nodes.get(destination) {
            if !replace { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "fixture target exists")); }
            if source_node.lock().unwrap().directory || target.lock().unwrap().directory { return Err(invalid()); }
        }
        let prefix = format!("{source}/");
        let paths = nodes.keys().filter(|path| path.as_str() == source || path.starts_with(&prefix))
            .cloned().collect::<Vec<_>>();
        nodes.remove(destination);
        for old_path in paths {
            let object = nodes.remove(&old_path).unwrap();
            nodes.insert(format!("{destination}{}", &old_path[source.len()..]), object);
        }
        Ok(())
    }
}

impl Backend for OptimizationBackend {
    fn scheme(&self) -> Scheme { Scheme::Sftp }
    fn root_display(&self) -> String { "/".into() }
    fn parallelism(&self) -> usize { 8 }
    fn case_sensitive_paths(&self, _: &str) -> bool { true }
    fn root_confinement(&self, _: &str) -> RootConfinement { RootConfinement::Enforced }
    fn rename_overwrites(&self) -> bool { true }
    fn staged_write_capabilities(&self, _: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities::complete()
    }

    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        self.0.stats.fetch_add(1, Ordering::SeqCst);
        let result = self.lookup(path).map(|node| {
            let state = node.lock().unwrap();
            metadata(path, &state)
        });
        let gate = {
            let mut pending = self.0.stat_gate.lock().unwrap();
            match pending.as_mut() {
                Some(gate) if gate.path == path && gate.skip == 0 => pending.take(),
                Some(gate) if gate.path == path => { gate.skip -= 1; None }
                _ => None,
            }
        };
        if let Some(gate) = gate {
            gate.entered.send(()).map_err(|_| io::Error::other("fixture gate owner disappeared"))?;
            gate.release.recv_timeout(Duration::from_secs(15))
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "fixture stat gate expired"))?;
        }
        result
    }

    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        let directory = self.lookup(path)?;
        if !directory.lock().unwrap().directory { return Err(invalid()); }
        let nodes = self.0.nodes.lock().unwrap();
        Ok(nodes.iter().filter(|(child, _)| child.as_str() != path && parent(child) == path)
            .map(|(child, node)| metadata(child, &node.lock().unwrap())).collect())
    }

    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        self.open_read_id(path, None)
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> io::Result<Box<dyn Read + Send>> {
        let node = self.lookup(path)?;
        let node = node.lock().unwrap();
        if node.directory || node.link { return Err(invalid()); }
        if id.is_some_and(|id| id != format!("optimization-{}", node.id)) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "fixture object identity changed"));
        }
        self.0.reads.fetch_add(1, Ordering::SeqCst);
        let bytes = self.0.read_override.lock().unwrap().take().unwrap_or_else(|| node.bytes.clone());
        Ok(Box::new(FixtureReader { inner: Arc::clone(&self.0), bytes: Cursor::new(bytes) }))
    }

    fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> { self.writer(path, false) }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> { self.writer(path, true) }
    fn rename(&self, src: &str, dst: &str) -> io::Result<()> { self.move_node(src, dst, true) }
    fn rename_no_replace(&self, src: &str, dst: &str) -> io::Result<()> { self.move_node(src, dst, false) }
    fn promote_staged(&self, src: &str, dst: &str) -> io::Result<()> { self.move_node(src, dst, true) }
    fn promote_staged_no_replace(&self, src: &str, dst: &str) -> io::Result<()> { self.move_node(src, dst, false) }

    fn remove_file(&self, path: &str) -> io::Result<()> {
        let mut nodes = self.0.nodes.lock().unwrap();
        let node = nodes.get(path).ok_or_else(missing)?;
        if node.lock().unwrap().directory { return Err(invalid()); }
        nodes.remove(path);
        Ok(())
    }

    fn remove_file_id(&self, path: &str, id: Option<&str>) -> io::Result<()> {
        let mut nodes = self.0.nodes.lock().unwrap();
        let node = nodes.get(path).ok_or_else(missing)?.lock().unwrap();
        if node.directory || id.is_some_and(|id| id != format!("optimization-{}", node.id)) { return Err(invalid()); }
        drop(node);
        nodes.remove(path);
        Ok(())
    }

    fn remove_dir(&self, path: &str) -> io::Result<()> {
        if path == "/" { return Err(invalid()); }
        let mut nodes = self.0.nodes.lock().unwrap();
        if !nodes.get(path).ok_or_else(missing)?.lock().unwrap().directory { return Err(invalid()); }
        if nodes.keys().any(|child| child.starts_with(&format!("{path}/"))) {
            return Err(io::Error::new(io::ErrorKind::DirectoryNotEmpty, "fixture directory is not empty"));
        }
        nodes.remove(path);
        Ok(())
    }

    fn mkdir_all(&self, path: &str) -> io::Result<()> {
        validate(path)?;
        let mut nodes = self.0.nodes.lock().unwrap();
        let mut current = String::new();
        for component in std::iter::once("").chain(path.split('/').filter(|part| !part.is_empty())) {
            if current != "/" { current.push('/'); }
            current.push_str(component);
            if let Some(node) = nodes.get(&current) {
                if !node.lock().unwrap().directory { return Err(invalid()); }
            } else {
                nodes.insert(current.clone(), self.node(true));
            }
        }
        Ok(())
    }
}

struct FixtureReader { inner: Arc<Inner>, bytes: Cursor<Vec<u8>> }
impl Read for FixtureReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let read = self.bytes.read(output)?;
        self.inner.streamed_bytes.fetch_add(read, Ordering::SeqCst);
        Ok(read)
    }
}

struct FixtureWriter { inner: Arc<Inner>, node: Arc<Mutex<Node>> }
impl Write for FixtureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.flush()?;
        let mut node = self.node.lock().unwrap();
        node.bytes.extend_from_slice(bytes);
        node.modified = self.inner.clock.fetch_add(1, Ordering::SeqCst);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.inner.fail_upload.load(Ordering::SeqCst) {
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "injected upload failure"))
        } else { Ok(()) }
    }
}

pub(crate) struct FixtureGate { entered: mpsc::Receiver<()>, release: Option<mpsc::Sender<()>> }
impl FixtureGate {
    pub(crate) fn wait(&self) { self.entered.recv_timeout(Duration::from_secs(10)).expect("fixture gate reached"); }
    pub(crate) fn release(mut self) { self.release.take().unwrap().send(()).expect("fixture gate released"); }
}
impl Drop for FixtureGate {
    fn drop(&mut self) { if let Some(release) = self.release.take() { let _ = release.send(()); } }
}

pub(crate) struct FixtureDirectory(PathBuf);
impl FixtureDirectory {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mount-optimization-{}", super::MountId::new_random().unwrap()));
        std::fs::create_dir(&path).expect("create owned fixture directory");
        Self(path)
    }
    pub(crate) fn path(&self) -> &Path { &self.0 }
}
impl Drop for FixtureDirectory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(parent, _)| if parent.is_empty() { "/" } else { parent }).unwrap_or("/")
}
fn metadata(path: &str, node: &Node) -> VfsMeta {
    VfsMeta { name: path.rsplit('/').next().unwrap_or("").into(), is_dir: node.directory,
        is_symlink: node.link, size: node.bytes.len() as u64, mtime_ms: node.modified,
        id: Some(format!("optimization-{}", node.id)), ..VfsMeta::default() }
}
fn require_parent(nodes: &BTreeMap<String, Arc<Mutex<Node>>>, path: &str) -> io::Result<()> {
    let node = nodes.get(parent(path)).ok_or_else(missing)?.lock().unwrap();
    if node.directory && !node.link { Ok(()) } else { Err(invalid()) }
}
fn validate(path: &str) -> io::Result<()> {
    if path == "/" || (path.starts_with('/') && !path.ends_with('/') && !path.contains('\\')
        && path[1..].split('/').all(|part| !part.is_empty() && !matches!(part, "." | ".."))) {
        Ok(())
    } else { Err(invalid()) }
}
fn missing() -> io::Error { io::Error::new(io::ErrorKind::NotFound, "fixture path is absent") }
fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, "invalid fixture namespace operation") }
