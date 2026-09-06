//! Daemon snapshot sharing, selected by the single remote vault task suite.
use super::{cache_index, cache_load::DirectorySnapshot, CacheLimits, CachingBackend};
use crate::vfs::{Backend, Scheme, VfsMeta};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(5);

struct Task<T> { done: mpsc::Receiver<T>, thread: std::thread::JoinHandle<()> }
impl<T: Send + 'static> Task<T> {
    fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (send, done) = mpsc::channel();
        let thread = std::thread::spawn(move || { let _ = send.send(work()); });
        Self { done, thread }
    }
    fn finish(self) -> T {
        // Join only after completion. A broken cache mutex cannot hang the
        // harness: its owned worker is detached when the deadline fails.
        let result = self.done.recv_timeout(DEADLINE).expect("daemon cache worker completed");
        self.thread.join().expect("daemon cache worker did not panic");
        result
    }
}

#[derive(Default)]
struct Gate { state: Mutex<(bool, bool)>, wake: Condvar }
impl Gate {
    fn block(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.wake.notify_all();
        let (state, _) = self.wake.wait_timeout_while(state, DEADLINE, |state| !state.1).unwrap();
        if state.1 { Ok(()) } else { Err(io::Error::new(io::ErrorKind::TimedOut, "fixture release expired")) }
    }
    fn wait(&self) {
        let state = self.state.lock().unwrap();
        let (state, _) = self.wake.wait_timeout_while(state, DEADLINE, |state| !state.0).unwrap();
        assert!(state.0, "backend listing reached its gate");
    }
    fn release(&self) { self.state.lock().unwrap().1 = true; self.wake.notify_all(); }
}
struct Release(Arc<Gate>);
impl Release { fn wait(&self) { self.0.wait(); } fn release(&self) { self.0.release(); } }
impl Drop for Release { fn drop(&mut self) { self.0.release(); } }

#[derive(Clone)]
enum Answer { Entries(Vec<VfsMeta>), Failure(io::ErrorKind, String) }
#[derive(Default)]
struct ListingBackend {
    answers: Mutex<HashMap<String, Answer>>,
    calls: Mutex<HashMap<String, usize>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
impl ListingBackend {
    fn entries(&self, path: &str, names: &[&str]) {
        self.answers.lock().unwrap().insert(path.into(), Answer::Entries(names.iter()
            .map(|name| VfsMeta { name: (*name).into(), is_dir: true,
                id: Some(format!("fixture-{name}")), ..VfsMeta::default() }).collect()));
    }
    fn gate(&self, path: &str) -> Release {
        let gate = Arc::new(Gate::default());
        assert!(self.gates.lock().unwrap().insert(path.into(), Arc::clone(&gate)).is_none());
        Release(gate)
    }
    fn count(&self, path: &str) -> usize { self.calls.lock().unwrap().get(path).copied().unwrap_or(0) }
}
impl Backend for ListingBackend {
    fn scheme(&self) -> Scheme { Scheme::Peer }
    fn root_display(&self) -> String { "/".into() }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        *self.calls.lock().unwrap().entry(path.into()).or_default() += 1;
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        let _active = Active(&self.active);
        let answer = self.answers.lock().unwrap().get(path).cloned()
            .unwrap_or(Answer::Failure(io::ErrorKind::NotFound, "fixture directory missing".into()));
        let gate = self.gates.lock().unwrap().remove(path);
        if let Some(gate) = gate { gate.block()?; }
        match answer {
            Answer::Entries(entries) => Ok(entries),
            Answer::Failure(kind, message) => Err(io::Error::new(kind, message)),
        }
    }
    fn remove_file(&self, path: &str) -> io::Result<()> {
        let (parent, name) = path.rsplit_once('/').unwrap();
        let mut answers = self.answers.lock().unwrap();
        let Some(Answer::Entries(entries)) = answers.get_mut(parent) else { return unused(); };
        entries.retain(|entry| entry.name != name);
        Ok(())
    }
    fn stat(&self, _: &str) -> io::Result<VfsMeta> { unused() }
    fn open_read(&self, _: &str) -> io::Result<Box<dyn Read + Send>> { unused() }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> { unused() }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> { unused() }
    fn remove_dir(&self, _: &str) -> io::Result<()> { unused() }
    fn mkdir_all(&self, _: &str) -> io::Result<()> { unused() }
}
fn unused<T>() -> io::Result<T> { Err(io::Error::new(io::ErrorKind::Unsupported, "unused fixture operation")) }

fn cache(inner: &Arc<ListingBackend>, retained: bool) -> Arc<CachingBackend> {
    let mut cache = CachingBackend::for_mount(inner.clone(), Some(|name| name.to_ascii_lowercase()));
    if !retained { cache.limits = CacheLimits { directories: usize::MAX, entries: usize::MAX, bytes: 1 }; }
    Arc::new(cache)
}
fn load(cache: &Arc<CachingBackend>, path: &str) -> Task<io::Result<DirectorySnapshot>> {
    let cache = Arc::clone(cache);
    let path = path.to_string();
    Task::spawn(move || cache.directory_snapshot(&path))
}
fn wait_for_owners(cache: &CachingBackend, path: &str, expected: usize) {
    let end = Instant::now() + DEADLINE;
    loop {
        // Weak::strong_count does not itself acquire a slot. Every counted
        // owner has passed production slot admission before leader release.
        let owners = cache.cache.lock().unwrap().loads.get(path).map_or(0, |slot| slot.strong_count());
        if owners >= expected { return; }
        assert!(Instant::now() < end, "expected {expected} slot owners, observed {owners}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn mount_vault_task_daemon_waiters_share_snapshot_and_index_without_retention() {
    for retained in [true, false] {
        let inner = Arc::new(ListingBackend::default());
        inner.entries("/ancestor", &["One", "Two"]);
        let cache = cache(&inner, retained);
        let release = inner.gate("/ancestor");
        let mut tasks = vec![load(&cache, "/ancestor")];
        release.wait();
        for _ in 1..8 { tasks.push(load(&cache, "/ancestor")); }
        wait_for_owners(&cache, "/ancestor", tasks.len());
        release.release();
        let snapshots = tasks.into_iter().map(|task| task.finish().unwrap()).collect::<Vec<_>>();
        assert_eq!(inner.count("/ancestor"), 1);
        for snapshot in &snapshots {
            assert!(Arc::ptr_eq(&snapshot.entries, &snapshots[0].entries));
            assert!(Arc::ptr_eq(&snapshot.index, &snapshots[0].index));
            assert_eq!(cache_index::lookup(&snapshot.entries, &snapshot.index, "two")
                .unwrap().unwrap().name, "Two");
        }
        let observation = Arc::downgrade(&snapshots[0].entries);
        let index = Arc::downgrade(&snapshots[0].index);
        drop(snapshots);
        {
            let state = cache.cache.lock().unwrap();
            assert!(state.loads.is_empty(), "last waiter removes its weak slot registration");
            assert_eq!(state.directories.len(), if retained { 1 } else { 0 });
        }
        if !retained {
            assert!(observation.upgrade().is_none());
            assert!(index.upgrade().is_none());
        }
        assert_eq!(cache.unique_child("/ancestor", "ONE").unwrap().unwrap().name, "One");
        assert_eq!(inner.count("/ancestor"), if retained { 1 } else { 2 });
        eprintln!("vault daemon sharing: retained={retained}, burst=8, burst_lists=1, shared_index=true");
    }
}

#[test]
fn mount_vault_task_daemon_unrelated_snapshot_loads_overlap() {
    let inner = Arc::new(ListingBackend::default());
    inner.entries("/one", &["a"]);
    inner.entries("/two", &["b"]);
    let cache = cache(&inner, true);
    let one = inner.gate("/one");
    let two = inner.gate("/two");
    let first = load(&cache, "/one");
    one.wait();
    let second = load(&cache, "/two");
    two.wait();
    assert_eq!(inner.maximum.load(Ordering::SeqCst), 2);
    one.release();
    two.release();
    assert_eq!(first.finish().unwrap().entries[0].name, "a");
    assert_eq!(second.finish().unwrap().entries[0].name, "b");
    assert_eq!(inner.active.load(Ordering::SeqCst), 0);
    assert!(cache.cache.lock().unwrap().loads.is_empty());
}

#[test]
fn mount_vault_task_daemon_mutation_fences_persistent_and_waiter_authority() {
    for retained in [true, false] {
        let inner = Arc::new(ListingBackend::default());
        inner.entries("/ancestor", &["old"]);
        let cache = cache(&inner, retained);
        let release = inner.gate("/ancestor");
        let leader = load(&cache, "/ancestor");
        release.wait();
        let waiter = load(&cache, "/ancestor");
        wait_for_owners(&cache, "/ancestor", 2);
        cache.remove_file("/ancestor/old").unwrap();
        release.release();
        // The initiating caller may receive its observation, but that stale
        // generation is not authority for a waiting or subsequent caller.
        assert_eq!(leader.finish().unwrap().entries.len(), 1);
        assert!(waiter.finish().unwrap().entries.is_empty());
        assert_eq!(inner.count("/ancestor"), 2);
        assert!(cache.unique_child("/ancestor", "old").unwrap().is_none());
        assert_eq!(inner.count("/ancestor"), if retained { 2 } else { 3 });
        assert!(cache.cache.lock().unwrap().loads.is_empty());
    }
}

#[test]
fn mount_vault_task_daemon_waiters_share_errors_but_do_not_retain_them() {
    let inner = Arc::new(ListingBackend::default());
    inner.answers.lock().unwrap().insert("/ancestor".into(),
        Answer::Failure(io::ErrorKind::PermissionDenied, "fixture access denied".into()));
    let cache = cache(&inner, true);
    let release = inner.gate("/ancestor");
    let mut tasks = vec![load(&cache, "/ancestor")];
    release.wait();
    for _ in 1..4 { tasks.push(load(&cache, "/ancestor")); }
    wait_for_owners(&cache, "/ancestor", tasks.len());
    release.release();
    for task in tasks {
        let error = task.finish().err().expect("shared failure");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "fixture access denied");
    }
    assert_eq!(inner.count("/ancestor"), 1);
    {
        let state = cache.cache.lock().unwrap();
        assert!(state.loads.is_empty());
        assert!(state.directories.is_empty());
    }
    inner.entries("/ancestor", &["available"]);
    assert_eq!(cache.unique_child("/ancestor", "available").unwrap().unwrap().name, "available");
    assert_eq!(inner.count("/ancestor"), 2);
}
