//! Work-conserving scheduling and joined teardown for the remote vault suite.
use super::metadata_batch::{run_metadata_batch, run_metadata_batch_keyed};
use super::metadata_cache::MetadataCache;
use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Start, End, StopObserved }
#[derive(Clone, Debug)]
struct Event { path: String, depth: u8, phase: Phase }
#[derive(Clone, Copy)]
enum Fault { Error, Panic }
#[derive(Default)]
struct Gate { released: Mutex<bool>, wake: Condvar }
impl Gate {
    fn wait(&self) -> io::Result<()> {
        let released = self.released.lock().unwrap();
        let (released, _) = self.wake.wait_timeout_while(released, DEADLINE, |ready| !*ready).unwrap();
        if *released { Ok(()) } else { Err(io::Error::new(io::ErrorKind::TimedOut, "scheduler gate expired")) }
    }
    fn release(&self) { *self.released.lock().unwrap() = true; self.wake.notify_all(); }
}
struct Release(Arc<Gate>);
impl Release { fn release(&self) { self.0.release(); } }
impl Drop for Release { fn drop(&mut self) { self.0.release(); } }

#[derive(Default)]
struct Trace {
    events: Mutex<Vec<Event>>,
    wake: Condvar,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    faults: Mutex<HashMap<String, Fault>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
}
struct Active<'a> { trace: &'a Trace, path: &'a str, depth: u8 }
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.trace.active.fetch_sub(1, Ordering::SeqCst);
        self.trace.record(self.path, self.depth, Phase::End);
    }
}
impl Trace {
    fn record(&self, path: &str, depth: u8, phase: Phase) {
        self.events.lock().unwrap().push(Event { path: path.into(), depth, phase });
        self.wake.notify_all();
    }
    fn gate(&self, path: &str) -> Release {
        let gate = Arc::new(Gate::default());
        assert!(self.gates.lock().unwrap().insert(path.into(), Arc::clone(&gate)).is_none());
        Release(gate)
    }
    fn work(&self, path: &str, depth: u8) -> io::Result<bool> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        let _active = Active { trace: self, path, depth };
        self.record(path, depth, Phase::Start);
        let gate = self.gates.lock().unwrap().get(path).cloned();
        if let Some(gate) = gate { gate.wait()?; }
        let fault = self.faults.lock().unwrap().get(path).copied();
        match fault {
            Some(Fault::Error) => Err(io::Error::other("injected scheduler work failure")),
            Some(Fault::Panic) => panic!("injected scheduler work panic"),
            None => Ok(true),
        }
    }
    fn wait_for(&self, condition: impl Fn(&[Event]) -> bool) {
        let events = self.events.lock().unwrap();
        let (events, _) = self.wake.wait_timeout_while(events, DEADLINE,
            |events| !condition(events)).unwrap();
        assert!(condition(&events), "scheduler condition not reached; events={events:?}");
    }
    fn ended(&self, path: &str) {
        self.wait_for(|events| events.iter().any(|event| event.path == path && event.phase == Phase::End));
    }
    fn started(&self, path: &str) {
        self.wait_for(|events| events.iter().any(|event| event.path == path && event.phase == Phase::Start));
    }
    fn before(&self, parent: &str, child: &str) {
        let events = self.events.lock().unwrap();
        let ended = events.iter().position(|event| event.path == parent && event.phase == Phase::End).unwrap();
        let began = events.iter().position(|event| event.path == child && event.phase == Phase::Start).unwrap();
        assert!(ended < began, "{parent} must finish before {child} starts: {events:?}");
    }
    fn starts(&self) -> usize {
        self.events.lock().unwrap().iter().filter(|event| event.phase == Phase::Start).count()
    }
}

struct Running { done: mpsc::Receiver<io::Result<usize>>, thread: std::thread::JoinHandle<()> }
impl Running {
    fn spawn(work: impl FnOnce() -> io::Result<usize> + Send + 'static) -> Self {
        let (send, done) = mpsc::channel();
        let thread = std::thread::spawn(move || { let _ = send.send(work()); });
        Self { done, thread }
    }
    fn finish(self) -> io::Result<usize> {
        // No scoped auto-join on timeout: even a broken scheduler cannot keep
        // this test waiting forever. Workers own their fixtures through Arcs.
        let result = self.done.recv_timeout(DEADLINE).expect("scheduler returned before deadline");
        self.thread.join().expect("scheduler outer worker did not panic");
        result
    }
}
fn batch(trace: &Arc<Trace>, targets: Vec<(String, u8)>, width: usize) -> Running {
    let trace = Arc::clone(trace);
    Running::spawn(move || run_metadata_batch(targets, width, &|| false,
        &|path, depth| trace.work(path, depth)))
}
fn targets(paths: &[(&str, u8)]) -> Vec<(String, u8)> {
    paths.iter().map(|(path, depth)| ((*path).into(), *depth)).collect()
}

#[test]
fn mount_vault_task_scheduler_uses_more_than_four_available_workers() {
    let trace = Arc::new(Trace::default());
    let selected = (0..6).map(|index| (format!("/w{index}"), 1)).collect::<Vec<_>>();
    let releases = selected.iter().map(|(path, _)| trace.gate(path)).collect::<Vec<_>>();
    let running = batch(&trace, selected, 6);
    trace.wait_for(|events| events.iter().filter(|event| event.phase == Phase::Start).count() == 6);
    assert_eq!(trace.maximum.load(Ordering::SeqCst), 6);
    for release in releases { release.release(); }
    assert_eq!(running.finish().unwrap(), 6);
    assert_eq!(trace.active.load(Ordering::SeqCst), 0);
    eprintln!("vault scheduler: requested_width=6, measured_overlap=6");
}

#[test]
fn mount_vault_task_scheduler_stalled_sibling_allows_other_depth_and_refill() {
    let trace = Arc::new(Trace::default());
    let release = trace.gate("/slow");
    let running = batch(&trace, targets(&[("/slow", 1), ("/a", 7), ("/a/child", 0),
        ("/unselected/deep/leaf", 9), ("/refill-one", 1), ("/refill-two", 1)]), 2);
    trace.started("/slow");
    for path in ["/a/child", "/unselected/deep/leaf", "/refill-one", "/refill-two"] { trace.ended(path); }
    assert_eq!(trace.active.load(Ordering::SeqCst), 1, "the stalled worker is still held");
    trace.before("/a", "/a/child");
    release.release();
    assert_eq!(running.finish().unwrap(), 6);
    assert_eq!(trace.maximum.load(Ordering::SeqCst), 2);
    assert_eq!(trace.active.load(Ordering::SeqCst), 0);
}

#[test]
fn mount_vault_task_scheduler_deduplicates_keyed_paths_with_boundary_ancestry() {
    let trace = Arc::new(Trace::default());
    let release = trace.gate("/A");
    let work = Arc::clone(&trace);
    let running = Running::spawn(move || run_metadata_batch_keyed(targets(&[
        ("/A", 7), ("/a", 0), ("/a/child", 0), ("/A/middle/deep", 9),
        ("/AB/leaf", 1), ("/İ", 8), ("/i\u{307}", 1), ("/İ/Child", 0),
    ]), 2, &|| false, &|path, depth| work.work(path, depth), &|path| path.to_lowercase()));
    trace.started("/A");
    trace.ended("/AB/leaf"); // /A is not an ancestor of /AB.
    trace.ended("/İ/Child"); // Key byte lengths may differ from original UTF-8 paths.
    {
        let events = trace.events.lock().unwrap();
        assert!(!events.iter().any(|event| event.path == "/a/child" || event.path == "/A/middle/deep"));
        assert!(events.iter().any(|event| event.path == "/A" && event.depth == 7));
        assert!(!events.iter().any(|event| event.path == "/a" || event.path == "/i\u{307}"));
    }
    release.release();
    assert_eq!(running.finish().unwrap(), 6);
    trace.before("/A", "/a/child");
    trace.before("/A", "/A/middle/deep");
    trace.before("/İ", "/İ/Child");
    assert_eq!(trace.active.load(Ordering::SeqCst), 0);
}

#[test]
fn mount_vault_task_scheduler_stop_joins_started_work_without_dispatching_children() {
    let trace = Arc::new(Trace::default());
    let stopped = Arc::new(AtomicBool::new(false));
    let release = trace.gate("/held");
    let work = Arc::clone(&trace);
    let stop = Arc::clone(&stopped);
    let running = Running::spawn(move || run_metadata_batch(targets(&[("/held", 1), ("/held/child", 2)]),
        2, &|| {
            let stopped = stop.load(Ordering::SeqCst);
            if stopped { work.record("stop", 0, Phase::StopObserved); }
            stopped
        }, &|path, depth| work.work(path, depth)));
    trace.started("/held");
    stopped.store(true, Ordering::SeqCst);
    trace.wait_for(|events| events.iter().any(|event| event.phase == Phase::StopObserved));
    assert_eq!(trace.active.load(Ordering::SeqCst), 1);
    assert!(matches!(running.done.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.release();
    assert_eq!(running.finish().unwrap(), 1);
    assert_eq!(trace.starts(), 1);
    assert_eq!(trace.active.load(Ordering::SeqCst), 0);
    let never = Arc::new(Trace::default());
    let work = Arc::clone(&never);
    let cancelled = Running::spawn(move || run_metadata_batch(targets(&[("/never", 0)]),
        6, &|| true, &|path, depth| work.work(path, depth)));
    assert_eq!(cancelled.finish().unwrap(), 0);
    assert_eq!(never.starts(), 0);
}

#[test]
fn mount_vault_task_scheduler_work_errors_and_panics_join_and_do_not_starve() {
    for fault in [Fault::Error, Fault::Panic] {
        let trace = Arc::new(Trace::default());
        trace.faults.lock().unwrap().insert("/failed".into(), fault);
        let release = trace.gate("/held");
        let running = batch(&trace, targets(&[("/held", 1), ("/failed", 1),
            ("/failed/child", 2), ("/later", 1)]), 2);
        trace.started("/held");
        trace.ended("/failed/child");
        trace.ended("/later");
        assert_eq!(trace.active.load(Ordering::SeqCst), 1);
        assert!(matches!(running.done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.release();
        let error = running.finish().unwrap_err();
        assert!(error.to_string().contains(match fault {
            Fault::Error => "injected scheduler work failure", Fault::Panic => "worker panicked",
        }));
        assert_eq!(trace.starts(), 4);
        assert_eq!(trace.active.load(Ordering::SeqCst), 0);
        trace.before("/failed", "/failed/child");
    }
}

#[test]
fn mount_vault_task_scheduler_panicking_stop_predicate_wakes_and_joins_workers() {
    let trace = Arc::new(Trace::default());
    let panic_stop = Arc::new(AtomicBool::new(false));
    let release = trace.gate("/held");
    let work = Arc::clone(&trace);
    let stop = Arc::clone(&panic_stop);
    let running = Running::spawn(move || run_metadata_batch(targets(&[("/held", 1), ("/held/child", 2)]),
        2, &|| {
            if stop.load(Ordering::SeqCst) {
                work.record("stop-panic", 0, Phase::StopObserved);
                panic!("injected stop predicate panic");
            }
            false
        }, &|path, depth| work.work(path, depth)));
    trace.started("/held");
    panic_stop.store(true, Ordering::SeqCst);
    trace.wait_for(|events| events.iter().any(|event| event.phase == Phase::StopObserved));
    assert_eq!(trace.active.load(Ordering::SeqCst), 1);
    release.release();
    assert!(running.finish().unwrap_err().to_string().contains("worker panicked"));
    assert_eq!(trace.starts(), 1);
    assert_eq!(trace.active.load(Ordering::SeqCst), 0);
}

#[test]
fn mount_vault_task_scheduler_foreground_refresh_satisfies_selected_revision() {
    let cache = MetadataCache::new("/", true);
    let directory = VfsMeta { is_dir: true, ..VfsMeta::default() };
    assert!(cache.install_directory("/", directory.clone(), Vec::<VfsMeta>::new().into(), 0).unwrap());
    let selected = cache.refresh_targets_with_revisions(1, true).unwrap();
    assert_eq!(selected.len(), 1);
    let (path, depth, revision) = &selected[0];
    assert!(!cache.refreshed_since(path, *revision).unwrap());
    assert!(cache.install_directory("/", directory, vec![VfsMeta {
        name: "new-note".into(), ..VfsMeta::default()
    }].into(), *depth).unwrap());
    assert!(cache.refreshed_since(path, *revision).unwrap());
}

#[test]
fn mount_vault_task_scheduler_recent_directory_survives_old_demand_backlog() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let directory = VfsMeta { is_dir: true, ..VfsMeta::default() };
    let names = (0..32).map(|index| format!("d{index:02}"))
        .chain(std::iter::once("watch".into())).collect::<Vec<_>>();
    let children = names.iter().map(|name| VfsMeta {
        name: name.clone(), ..directory.clone()
    }).collect::<Vec<_>>();
    assert!(cache.install_directory("/", directory.clone(), children.into(), 0)?);
    for name in &names {
        let path = format!("/{name}");
        assert!(cache.install_directory(&path, directory.clone(), Vec::new().into(), 1)?);
        cache.mark_directory_access(&path)?;
    }
    // Simulate the existing pre-watch refresh. No new directory reads occur
    // after this point; its last demand is serviced but it remains most recent.
    assert!(cache.install_directory("/watch", directory, Vec::new().into(), 1)?);
    let selected = cache.refresh_targets(16, true)?;
    assert!(selected.iter().any(|(path, _)| path == "/watch"));
    let mut visited = std::collections::HashSet::new();
    for _ in 0..names.len() {
        let selected = cache.refresh_targets(3, true)?;
        assert_eq!(selected.len(), 3);
        assert!(selected.iter().any(|(path, _)| path == "/watch"));
        visited.extend(selected.into_iter().map(|(path, _)| path));
    }
    assert!(names.iter().all(|name| visited.contains(&format!("/{name}"))),
        "reserving recent demand must retain cold-attempt fairness");
    Ok(())
}
