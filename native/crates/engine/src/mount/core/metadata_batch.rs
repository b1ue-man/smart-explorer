//! Bounded background workers with dependencies only between selected ancestors.
use std::collections::{HashMap, VecDeque};
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

struct Target {
    path: String,
    depth: u8,
    children: Vec<usize>,
}

struct State {
    ready: VecDeque<usize>,
    remaining: usize,
    completed: usize,
    stopped: bool,
    first_error: Option<io::Error>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

/// Exact-path identity wrapper. Mount callers with case-insensitive identities
/// must use `run_metadata_batch_keyed` with their cache's identity function.
pub(in crate::mount) fn run_metadata_batch(
    targets: Vec<(String, u8)>,
    width: usize,
    stopped: &(impl Fn() -> bool + Sync),
    work: &(impl Fn(&str, u8) -> io::Result<bool> + Sync),
) -> io::Result<usize> {
    run_metadata_batch_keyed(targets, width, stopped, work, &|path| path.to_string())
}

/// Refill each worker as soon as its current target finishes. A selected child
/// waits for its closest selected ancestor; unrelated depths do not form waves.
/// `key` must preserve slash-separated ancestry and implement the caller's path
/// identity rules. Only keys are compared/sliced; work receives original paths,
/// even when Unicode case mapping changes a key's byte length. Depth is a work
/// hint, not an identity/dependency key. Duplicate identities run once, using
/// the first supplied original path, depth and priority.
///
/// Work errors (including panics) are collected without starving other selected
/// targets. Stop/spawn failure prevents further dispatch. Every started worker
/// is joined before returning; stopping cannot interrupt work already running.
pub(in crate::mount) fn run_metadata_batch_keyed(
    targets: Vec<(String, u8)>,
    width: usize,
    stopped: &(impl Fn() -> bool + Sync),
    work: &(impl Fn(&str, u8) -> io::Result<bool> + Sync),
    key: &(impl Fn(&str) -> String + Sync),
) -> io::Result<usize> {
    let (targets, ready) = prepare_targets(targets, key);
    if targets.is_empty() {
        return Ok(0);
    }
    let workers = width.max(1).min(targets.len());
    let shared = Shared {
        state: Mutex::new(State {
            ready,
            remaining: targets.len(),
            completed: 0,
            stopped: false,
            first_error: None,
        }),
        wake: Condvar::new(),
    };
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            match std::thread::Builder::new()
                .name("mount-metadata-load".into())
                .spawn_scoped(scope, || {
                    // In particular, a panicking stop predicate must wake
                    // workers waiting for a dependency that can no longer finish.
                    let outcome = catch_unwind(AssertUnwindSafe(|| {
                        run_worker(&targets, &shared, stopped, work);
                    }));
                    if outcome.is_err() {
                        shared.abort(io::Error::other("mounted metadata worker panicked"));
                    }
                })
            {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    shared.abort(error);
                    break;
                }
            }
        }
        for handle in handles {
            if handle.join().is_err() {
                shared.abort(io::Error::other("mounted metadata worker panicked"));
            }
        }
    });
    let mut state = shared.lock();
    state.first_error.take().map_or(Ok(state.completed), Err)
}

fn prepare_targets(
    targets: Vec<(String, u8)>,
    key: &(impl Fn(&str) -> String + Sync),
) -> (Vec<Target>, VecDeque<usize>) {
    let mut indexes = HashMap::with_capacity(targets.len());
    let mut identities = Vec::with_capacity(targets.len());
    let mut unique = Vec::with_capacity(targets.len());
    for (path, depth) in targets {
        let identity = key(&path);
        if indexes.contains_key(&identity) {
            continue;
        }
        indexes.insert(identity.clone(), unique.len());
        identities.push(identity);
        unique.push(Target { path, depth, children: Vec::new() });
    }
    let mut ready = VecDeque::new();
    for index in 0..unique.len() {
        if let Some(parent) = selected_parent(&identities[index], &indexes) {
            unique[parent].children.push(index);
        } else {
            ready.push_back(index);
        }
    }
    (unique, ready)
}

fn selected_parent(path: &str, indexes: &HashMap<String, usize>) -> Option<usize> {
    let mut ancestor = path;
    while ancestor != "/" {
        let boundary = ancestor.rfind('/')?;
        ancestor = if boundary == 0 { "/" } else { &ancestor[..boundary] };
        if let Some(index) = indexes.get(ancestor) {
            return Some(*index);
        }
    }
    None
}

fn run_worker(
    targets: &[Target],
    shared: &Shared,
    stopped: &(impl Fn() -> bool + Sync),
    work: &(impl Fn(&str, u8) -> io::Result<bool> + Sync),
) {
    loop {
        // Neither caller closure runs while the scheduler mutex is held.
        if stopped() {
            shared.stop();
            return;
        }
        let next = {
            let mut state = shared.lock();
            if state.stopped || state.remaining == 0 {
                return;
            }
            if let Some(index) = state.ready.pop_front() {
                Some(index)
            } else {
                // The caller's stop predicate has no associated notifier.
                // Poll only while idle behind active ancestors, never in work.
                let waited = shared.wake.wait_timeout(state, STOP_POLL_INTERVAL);
                drop(waited.unwrap_or_else(|poisoned| poisoned.into_inner()));
                None
            }
        };
        let Some(index) = next else { continue };
        if stopped() {
            shared.stop();
            return;
        }
        let target = &targets[index];
        let result = catch_unwind(AssertUnwindSafe(|| work(&target.path, target.depth)))
            .unwrap_or_else(|_| Err(io::Error::other("mounted metadata worker panicked")));
        let mut state = shared.lock();
        state.remaining -= 1;
        match result {
            Ok(true) => state.completed += 1,
            Ok(false) => {}
            Err(error) => { state.first_error.get_or_insert(error); }
        }
        if !state.stopped {
            state.ready.extend(target.children.iter().copied());
        }
        shared.wake.notify_all();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.stopped = true;
                state.first_error.get_or_insert_with(|| {
                    io::Error::other("mounted metadata scheduler is unavailable")
                });
                self.wake.notify_all();
                state
            }
        }
    }

    fn stop(&self) {
        self.lock().stopped = true;
        self.wake.notify_all();
    }

    fn abort(&self, error: io::Error) {
        let mut state = self.lock();
        state.stopped = true;
        state.first_error.get_or_insert(error);
        self.wake.notify_all();
    }
}
