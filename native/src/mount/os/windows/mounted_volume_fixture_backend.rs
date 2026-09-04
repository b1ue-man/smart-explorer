use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::{
    collections::BTreeMap,
    io::{self, Cursor, Read, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

pub(super) const CONTENTS: &[u8] = b"Smart Explorer mounted-volume fixture\n";
pub(super) const DEPTH: usize = 9;
const LINK: &str = "/outside-link";

/// Only the storage data is synthetic. Windows opens still pass through the
/// installed kernel driver, System32 DLL, production callbacks and mount engine.
pub(super) struct FixtureBackend {
    nodes: BTreeMap<String, VfsMeta>,
    stall: Mutex<Option<Instant>>,
    wake: Condvar,
    mutations: AtomicUsize,
    link_accesses: AtomicUsize,
    stalled_calls: AtomicUsize,
}

impl FixtureBackend {
    pub(super) fn new() -> Arc<Self> {
        let mut nodes = BTreeMap::new();
        insert(&mut nodes, "/", true);
        insert(&mut nodes, "/root.txt", false);
        for number in 0..8 {
            let directory = format!("/folder{number:02}");
            insert(&mut nodes, &directory, true);
            insert(&mut nodes, &format!("{directory}/note.txt"), false);
        }
        let mut directory = "/deep".to_string();
        insert(&mut nodes, &directory, true);
        insert(&mut nodes, &format!("{directory}/note.txt"), false);
        for depth in 1..=DEPTH {
            directory.push_str(&format!("/level{depth:02}"));
            insert(&mut nodes, &directory, true);
            insert(&mut nodes, &format!("{directory}/note.txt"), false);
        }
        insert(&mut nodes, LINK, true);
        nodes.get_mut(LINK).unwrap().is_symlink = true;
        Arc::new(Self {
            nodes,
            stall: Mutex::new(None),
            wake: Condvar::new(),
            mutations: AtomicUsize::new(0),
            link_accesses: AtomicUsize::new(0),
            stalled_calls: AtomicUsize::new(0),
        })
    }

    pub(super) fn expected_names(&self, path: &str) -> Vec<String> {
        let mut names = self
            .children(path)
            .filter(|metadata| !metadata.is_symlink)
            .map(|metadata| metadata.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub(super) fn arm_stall(self: &Arc<Self>) -> StallRelease {
        *self.stall.lock().unwrap() = Some(Instant::now() + Duration::from_secs(45));
        StallRelease(Arc::clone(self))
    }

    pub(super) fn release_stall(&self) {
        *self.stall.lock().unwrap_or_else(|error| error.into_inner()) = None;
        self.wake.notify_all();
    }

    pub(super) fn assert_read_only(&self) {
        assert_eq!(self.mutations.load(Ordering::SeqCst), 0, "backend mutation");
        assert_eq!(self.link_accesses.load(Ordering::SeqCst), 0, "link traversed");
    }

    pub(super) fn stalled_calls(&self) -> usize {
        self.stalled_calls.load(Ordering::SeqCst)
    }

    fn children<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a VfsMeta> + 'a {
        self.nodes.iter().filter_map(move |(candidate, metadata)| {
            let (parent, _) = candidate.rsplit_once('/')?;
            let parent = if parent.is_empty() { "/" } else { parent };
            (candidate != "/" && parent == path).then_some(metadata)
        })
    }

    fn await_release(&self) -> io::Result<()> {
        let mut state = self.stall.lock().map_err(|_| io::Error::other("stall poisoned"))?;
        if state.is_some() {
            self.stalled_calls.fetch_add(1, Ordering::SeqCst);
        }
        while let Some(deadline) = *state {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                *state = None;
                self.wake.notify_all();
                return Err(io::Error::new(io::ErrorKind::TimedOut, "fixture stall expired"));
            }
            state = self.wake.wait_timeout(state, remaining)
                .map_err(|_| io::Error::other("stall poisoned"))?.0;
        }
        Ok(())
    }

    fn reject_link(&self, path: &str) -> io::Result<()> {
        if path == LINK || path.starts_with("/outside-link/") {
            self.link_accesses.fetch_add(1, Ordering::SeqCst);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture link boundary"));
        }
        Ok(())
    }

    fn mutation<T>(&self) -> io::Result<T> {
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture is read-only"))
    }
}

pub(super) struct StallRelease(Arc<FixtureBackend>);

impl Drop for StallRelease {
    fn drop(&mut self) {
        self.0.release_stall();
    }
}

impl Backend for FixtureBackend {
    fn scheme(&self) -> Scheme { Scheme::Peer }
    fn root_display(&self) -> String { "/".into() }
    fn case_sensitive_paths(&self, _root: &str) -> bool { true }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.reject_link(path)?;
        self.await_release()?;
        match self.nodes.get(path) {
            Some(metadata) if metadata.is_dir => Ok(self.children(path).cloned().collect()),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "fixture directory missing")),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.await_release()?;
        self.nodes.get(path).cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture object missing"))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reject_link(path)?;
        self.await_release()?;
        match self.nodes.get(path) {
            Some(metadata) if !metadata.is_dir => Ok(Box::new(Cursor::new(CONTENTS))),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "fixture file missing")),
        }
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> { self.mutation() }
    fn open_write_new(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> { self.mutation() }
    fn copy_file(&self, _source: &str, _destination: &str) -> VfsResult<u64> { self.mutation() }
    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> { self.mutation() }
    fn rename_no_replace(&self, _source: &str, _destination: &str) -> VfsResult<()> { self.mutation() }
    fn promote_staged(&self, _source: &str, _destination: &str) -> VfsResult<()> { self.mutation() }
    fn promote_staged_no_replace(&self, _source: &str, _destination: &str) -> VfsResult<()> { self.mutation() }
    fn remove_file(&self, _path: &str) -> VfsResult<()> { self.mutation() }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> { self.mutation() }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> { self.mutation() }
}

fn insert(nodes: &mut BTreeMap<String, VfsMeta>, path: &str, is_dir: bool) {
    nodes.insert(path.into(), VfsMeta {
        name: path.rsplit('/').next().unwrap_or_default().to_string(),
        is_dir,
        size: if is_dir { 0 } else { CONTENTS.len() as u64 },
        mtime_ms: 1_700_000_000_000,
        ..VfsMeta::default()
    });
}
