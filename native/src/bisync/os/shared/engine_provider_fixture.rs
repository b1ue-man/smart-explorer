//! Test-only adapters for the committed engine boundary; no network transport.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::bisync as engine;
use crate::vfs::{Backend, BackendExtensions, ChangeKind, Scheme, VfsChange, VfsMeta};
pub(super) use engine::backend_identity_state::Binding as Identity;
pub(super) use engine::test_remote::{FakeRemote, REMOTE_ROOT};

#[derive(Clone, Copy)]
pub(super) enum Fault {
    AtomicBefore,
    AtomicAfter,
    NoReplaceMoved,
    NoReplacePublished,
    ForeignCreator,
    AtomicSuccess,
    NoReplaceSuccess,
}

pub(super) struct TestRemote {
    inner: FakeRemote,
    pub(super) identity: String,
    pub(super) previous: Vec<String>,
    pub(super) fault: Option<Fault>,
    pub(super) literal_root: Option<String>,
    pub(super) reject_archives: bool,
    pub(super) hooks: AtomicUsize,
    pub(super) promotions: AtomicUsize,
    pub(super) intent_pair: Mutex<Option<String>>,
    pub(super) requested: Mutex<Vec<String>>,
}

impl TestRemote {
    pub(super) fn new(base: &Path, host: &str) -> Self {
        let inner = FakeRemote::new(base, host);
        let identity = inner.state_identity();
        Self {
            inner,
            identity,
            previous: Vec::new(),
            fault: None,
            literal_root: None,
            reject_archives: false,
            hooks: AtomicUsize::new(0),
            promotions: AtomicUsize::new(0),
            intent_pair: Mutex::new(None),
            requested: Mutex::new(Vec::new()),
        }
    }

    fn path(&self, path: &str) -> String {
        let Some(root) = self.literal_root.as_deref() else { return path.to_string(); };
        let Some(relative) = path.strip_prefix(&format!("{root}/")) else { return path.to_string(); };
        let relative = relative.replace("%3A", ":").replace("%23", "#")
            .replace("%20", " ").replace("%25", "%");
        format!("{root}/{relative}")
    }
}

fn lost_ack() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "injected lost publish ACK")
}

impl Backend for TestRemote {
    fn scheme(&self) -> Scheme { self.inner.scheme() }
    fn root_display(&self) -> String { REMOTE_ROOT.to_string() }
    fn state_identity(&self) -> String { self.identity.clone() }
    fn extensions(&self) -> Option<&dyn BackendExtensions> { Some(self) }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        self.inner.list_dir(&self.path(path))
    }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        self.requested.lock().unwrap().push(path.to_string());
        self.inner.stat(&self.path(path))
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        self.inner.open_read(&self.path(path))
    }
    fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write(&self.path(path))
    }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write_new(&self.path(path))
    }
    fn rename(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename(&self.path(source), &self.path(destination))
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> io::Result<()> {
        if self.reject_archives && destination.contains("/.se-versions/") {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "fixture cannot archive by rename"));
        }
        self.inner.rename_no_replace(&self.path(source), &self.path(destination))
    }
    fn remove_file(&self, path: &str) -> io::Result<()> {
        self.inner.remove_file(&self.path(path))
    }
    fn remove_dir(&self, path: &str) -> io::Result<()> {
        self.inner.remove_dir(&self.path(path))
    }
    fn mkdir_all(&self, path: &str) -> io::Result<()> {
        self.inner.mkdir_all(&self.path(path))
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> io::Result<()> {
        self.promotions.fetch_add(1, Ordering::SeqCst);
        if matches!(self.fault, Some(Fault::AtomicBefore)) { return Err(lost_ack()); }
        self.inner.promote_staged(&self.path(staged), &self.path(destination))?;
        if matches!(self.fault, Some(Fault::AtomicAfter)) { return Err(lost_ack()); }
        Ok(())
    }
}

impl BackendExtensions for TestRemote {
    fn previous_state_identities(&self) -> io::Result<Vec<String>> { Ok(self.previous.clone()) }
    fn sync_child_path(&self, parent: &str, name: &str) -> io::Result<String> {
        let name = if self.literal_root.is_some() {
            name.replace('%', "%25").replace(':', "%3A")
                .replace(' ', "%20").replace('#', "%23")
        } else { name.to_string() };
        Ok(format!("{}/{name}", parent.trim_end_matches('/')))
    }
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> io::Result<Box<dyn Read + Send>> {
        crate::vfs::open_read_regular(&self.inner, &self.path(path), id)
    }
    fn finish_stage(&self, path: &str, finish: crate::vfs::StageFinish)
        -> io::Result<crate::vfs::StageFinished> {
        crate::vfs::finish_stage(&self.inner, &self.path(path), finish)
    }
    fn replace_staged_reversible(&self, staged: &str, destination: &str, retained: &str)
        -> io::Result<bool> {
        if let Some(pair) = self.intent_pair.lock().unwrap().as_deref() {
            let prepared = intent(pair);
            assert_eq!((prepared.stage.as_str(), prepared.destination.as_str(), prepared.retained.as_str()),
                (staged, destination, retained));
        }
        self.hooks.fetch_add(1, Ordering::SeqCst);
        if !matches!(self.fault, Some(Fault::NoReplaceMoved | Fault::NoReplacePublished
            | Fault::NoReplaceSuccess | Fault::ForeignCreator)) { return Ok(false); }
        self.rename_no_replace(destination, retained)?;
        if matches!(self.fault, Some(Fault::NoReplaceMoved)) { return Err(lost_ack()); }
        if matches!(self.fault, Some(Fault::ForeignCreator)) {
            let mut file = self.open_write_new(destination)?;
            file.write_all(b"foreign")?;
            file.flush()?;
            return Err(lost_ack());
        }
        self.rename_no_replace(staged, destination)?;
        if matches!(self.fault, Some(Fault::NoReplacePublished)) { return Err(lost_ack()); }
        Ok(true)
    }
}

/// Only fresh, uniquely identified fixture state is owned or cleaned up.
pub(super) struct StateFiles { paths: Vec<PathBuf> }
impl StateFiles {
    pub(super) fn new(bindings: &[Identity]) -> Self {
        let mut paths = Vec::new();
        for binding in bindings {
            paths.push(engine::replica_state::pair_dir(&binding.pair));
            paths.push(engine::persistence::versions_dir(&binding.pair));
            let base = engine::baseline_path(&binding.pair);
            for extension in ["sebl", "journal", "dirs.json", "spellings.json", "index-dirty"] {
                paths.push(base.with_extension(extension));
            }
        }
        paths.sort();
        paths.dedup();
        for path in &paths { assert!(!path.try_exists().unwrap(), "fixture state already exists"); }
        Self { paths }
    }
    pub(super) fn track(&mut self, path: PathBuf) {
        assert!(!path.try_exists().unwrap(), "fixture sidecar already exists");
        self.paths.push(path);
    }
}
impl Drop for StateFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            if path.is_dir() { let _ = std::fs::remove_dir_all(path); }
            else { let _ = std::fs::remove_file(path); }
        }
    }
}

pub(super) fn reverse(binding: &Identity) -> Identity {
    Identity::new([binding.identities[1].clone(), binding.identities[0].clone()],
        [binding.roots[1].clone(), binding.roots[0].clone()])
}

pub(super) fn key(binding: &Identity, owner: &str) -> engine::StateKey {
    engine::StateKey {
        pair_id: binding.pair.clone(), lock_id: binding.lock.clone(),
        owner: engine::StateOwner::Job(owner.into()),
        replica_a: engine::ReplicaRef::Marker("fixture-a".into()),
        replica_b: engine::ReplicaRef::Marker("fixture-b".into()),
    }
}

pub(super) fn signature(backend: &dyn Backend, path: &str) -> engine::Sig {
    engine::replacement_journal::observe(backend, path, &AtomicBool::new(false)).unwrap().unwrap().signature
}

pub(super) fn intent(pair: &str) -> engine::replacement_journal::Intent {
    let dir = engine::replica_state::pair_dir(pair);
    let path = std::fs::read_dir(dir).unwrap().filter_map(Result::ok).map(|entry| entry.path())
        .find(|path| path.file_name().unwrap().to_string_lossy().contains(".replace-")).unwrap();
    let text = crate::support_dirs::read_private_text(&path, 256 * 1024).unwrap();
    serde_json::from_str(&text).unwrap()
}

pub(super) fn merge(key: &engine::StateKey, rel: &str, sibling: Option<&str>) {
    let recovery = engine::merge_recovery::Recovery {
        pair: key.pair_id.clone(), lock: key.lock_id.clone(), rel: rel.into(), kind: "write".into(),
        original_a: None, original_b: None, merged: format!("{:x}", md5::compute(b"merged")),
        run: "fixture".into(), started_ms: 1, a: None, b: None, done_a: false, done_b: false,
        sibling: sibling.map(str::to_string), sibling_a: None, sibling_b: None,
    };
    engine::merge_recovery::save(key, &recovery).unwrap();
}
