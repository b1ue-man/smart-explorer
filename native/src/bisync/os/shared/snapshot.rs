use crate::transfer::{flow_for, Flow};
use crate::vfs::Backend;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::snapshot_dir::{scan_listing, Listed, WalkContext};
pub use super::snapshot_hash::HashMode;
pub(super) use super::snapshot_hash::{hash_mode, md5_hex_to_u64, md5_to_u64};
use super::sync_flows::{finish_listing, next_job};
use super::types::{Baseline, Tree};

pub(super) const MAX_WALK_NODES: u64 = 1_000_000;
pub(super) const MAX_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 512;
/// The coordinator looks at the flow at least this often (its limit moves
/// while other jobs use the connection).
const COORDINATOR_SLICE: Duration = Duration::from_millis(50);
/// An idle listing thread waits this long for the folders a busy one is
/// about to find: longer than one listing round trip on common links (the
/// transfer engine's reasoning), short against the walk.
const WORKER_LINGER: Duration = Duration::from_millis(250);

/// What to skip while walking: hidden files, ignore globs (matched on the
/// relative path), and size/age bounds (Group G). A bound of 0 means "no limit".
pub struct WalkFilter<'a> {
    pub include_hidden: bool,
    pub ignore: &'a globset::GlobSet,
    /// Only include files with `min_size <= size <= max_size` (bytes; 0 = off).
    pub min_size: u64,
    pub max_size: u64,
    /// Only include files modified within `[after_mtime_ms, before_mtime_ms]`
    /// (unix ms; 0 = off on that side).
    pub after_mtime_ms: i64,
    pub before_mtime_ms: i64,
}

impl<'a> WalkFilter<'a> {
    pub(super) fn ignored(&self, relative: &str, directory_like: bool) -> bool {
        self.ignore.is_match(relative)
            || (directory_like && self.ignore.is_match(format!("{relative}/")))
    }

    /// A filter with no size/age bounds (the common case).
    pub fn basic(include_hidden: bool, ignore: &'a globset::GlobSet) -> Self {
        WalkFilter {
            include_hidden,
            ignore,
            min_size: 0,
            max_size: 0,
            after_mtime_ms: 0,
            before_mtime_ms: 0,
        }
    }

    /// Does a file of this size/mtime pass the size & age bounds?
    pub(super) fn size_age_ok(&self, size: u64, mtime_ms: i64) -> bool {
        if self.min_size > 0 && size < self.min_size {
            return false;
        }
        if self.max_size > 0 && size > self.max_size {
            return false;
        }
        if self.after_mtime_ms > 0 && mtime_ms < self.after_mtime_ms {
            return false;
        }
        if self.before_mtime_ms > 0 && mtime_ms > self.before_mtime_ms {
            return false;
        }
        true
    }
}

/// An empty filter (include everything) — handy for tests / "no settings".
pub fn empty_globset() -> globset::GlobSet {
    globset::GlobSetBuilder::new().build().unwrap()
}

/// One side's last-known tree (rel → Sig) reconstructed from the saved baseline,
/// used by `walk_files` to reuse stored hashes for files whose size+mtime are
/// unchanged (so a large local tree isn't re-hashed on every run).
pub(super) fn prev_side(base: &Baseline, side_a: bool) -> Tree {
    base.iter()
        .filter_map(|(rel, (a, b))| (if side_a { *a } else { *b }).map(|s| (rel.clone(), s)))
        .collect()
}

/// Folders are listed concurrently: a remote side as many at once as its
/// connection's flow allows (listings may use the flow's reserved slot), a
/// local side on `parallelism()` threads (all cores) as before.
///
/// `hash` chooses the content-hash strategy (see `HashMode`). `prev` is the
/// previous run's tree for THIS side (from the saved baseline): when a file's
/// size+mtime are unchanged from `prev` we reuse its stored hash instead of
/// re-reading the file — so re-hashing a large local tree every sync is avoided.
pub fn walk_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
) -> io::Result<Tree> {
    walk_files_impl(be, root, cancel, filter, hash, prev, false, None)
}

/// Mirror destinations on ID-addressed providers may contain pre-existing
/// duplicate regular-file names. The caller must preflight and apply an exact
/// dedupe plan before any path-based writes; this walk only selects the same
/// deterministic newest ID for planning.
pub(super) fn walk_files_with_duplicate_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
) -> io::Result<Tree> {
    walk_files_impl(be, root, cancel, filter, hash, prev, true, None)
}

pub(super) struct Snapshot {
    pub tree: Tree,
    pub omissions: super::omissions::SyncOmissions,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn walk_snapshot(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
    allow_duplicate_files: bool,
    fold_case: bool,
) -> io::Result<Snapshot> {
    let omissions = Mutex::new(super::omissions::SyncOmissions::new(fold_case));
    let tree = walk_files_impl(
        be,
        root,
        cancel,
        filter,
        hash,
        prev,
        allow_duplicate_files,
        Some(&omissions),
    )?;
    Ok(Snapshot {
        tree,
        omissions: omissions.into_inner().unwrap_or_else(|e| e.into_inner()),
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_files_impl(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
    allow_duplicate_files: bool,
    omissions: Option<&Mutex<super::omissions::SyncOmissions>>,
) -> io::Result<Tree> {
    let canceled = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "synchronization tree walk canceled",
        )
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    // Fast path: when the backend can produce the signature SERVER-SIDE (the SSH
    // agent's WalkHashed), get the whole tree — including content MD5 for Full —
    // in one pass without downloading a single file. Falls through to the per-dir
    // walk if it didn't run.
    if be.supports_walk_hashed() {
        if let Some(tree) =
            super::snapshot_agent::walk_hashed_via_agent(be, root, cancel, filter, hash)?
        {
            return Ok(tree);
        }
    }

    let flow = (!be.is_local()).then(|| flow_for(be, root));
    let context = WalkContext {
        be,
        root,
        cancel,
        filter,
        hash,
        prev,
        allow_duplicate_files,
        omissions,
        nodes: AtomicU64::new(1),
        text_bytes: AtomicU64::new(root.len() as u64),
        reads: flow.clone().map(|flow| (flow, next_job())),
    };
    let walk = TreeWalk {
        context: &context,
        flow,
        local_threads: be.parallelism().max(1),
        state: Mutex::new(WalkState::default()),
        changed: Condvar::new(),
        out: Mutex::new(Tree::new()),
    };
    walk.lock().queue.push_back((root.to_string(), 0));
    std::thread::scope(|scope| walk.coordinate(scope));

    // A worker can observe cancellation while it is part-way through a
    // directory. Never turn that partial walk into a successful snapshot:
    // callers persist successful walks as the next deletion baseline.
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    let TreeWalk { state, out, .. } = walk;
    if let Some(error) = state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .error
    {
        return Err(error);
    }
    Ok(out
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner()))
}

fn list_plain_directory(be: &dyn Backend, path: &str) -> io::Result<Vec<crate::vfs::VfsMeta>> {
    let metadata = be.stat(path)?;
    if metadata.is_symlink || !metadata.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("sync directory changed into a link or non-directory: {path}"),
        ));
    }
    be.list_dir(path)
}

#[derive(Default)]
struct WalkState {
    /// Folders to list, with their depth below the root.
    queue: VecDeque<(String, usize)>,
    running: usize,
    busy: usize,
    /// Threads holding a folder but still waiting for a listing permit.
    waiting: usize,
    /// The first failure; it ends the walk.
    error: Option<io::Error>,
    done: bool,
}

/// The folders of one walk, listed by a pool that grows while folders wait:
/// up to the flow's limit plus its reserved listing slot on a remote side,
/// up to `parallelism()` threads on a local one.
struct TreeWalk<'a> {
    context: &'a WalkContext<'a>,
    flow: Option<Arc<Flow>>,
    local_threads: usize,
    state: Mutex<WalkState>,
    changed: Condvar,
    out: Mutex<Tree>,
}

impl TreeWalk<'_> {
    fn lock(&self) -> MutexGuard<'_, WalkState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'g>(
        &self,
        state: MutexGuard<'g, WalkState>,
        limit: Duration,
    ) -> MutexGuard<'g, WalkState> {
        match self.changed.wait_timeout(state, limit) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    fn canceled(&self) -> bool {
        self.context.cancel.load(Ordering::Relaxed)
    }

    fn fail(&self, error: io::Error) {
        let mut state = self.lock();
        if state.error.is_none() {
            state.error = Some(error);
        }
        drop(state);
        self.changed.notify_all();
    }

    fn coordinate<'s>(&'s self, scope: &'s std::thread::Scope<'s, '_>) {
        loop {
            let mut state = self.lock();
            let over = self.canceled()
                || state.error.is_some()
                || (state.queue.is_empty() && state.busy == 0);
            if over && !state.done {
                state.done = true;
                self.changed.notify_all();
            }
            if state.done {
                if state.running == 0 {
                    break;
                }
            } else if self.may_grow(&state) {
                state.running += 1;
                drop(state);
                if !self.spawn(scope) {
                    // Out of threads: look again after a pause, not in a
                    // busy loop.
                    drop(self.wait(self.lock(), COORDINATOR_SLICE));
                }
                continue;
            }
            drop(self.wait(state, COORDINATOR_SLICE));
        }
    }

    fn may_grow(&self, state: &WalkState) -> bool {
        let cap = match &self.flow {
            Some(flow) => flow.snapshot().limit.saturating_add(1),
            None => self.local_threads,
        };
        state.queue.len() > state.running.saturating_sub(state.busy)
            && state.waiting == 0
            && state.running < cap
    }

    /// Starts one listing thread; false when the system refused it.
    fn spawn<'s>(&'s self, scope: &'s std::thread::Scope<'s, '_>) -> bool {
        let spawned = std::thread::Builder::new()
            .name("sync-walk".to_string())
            .spawn_scoped(scope, move || self.worker());
        let Err(error) = spawned else {
            return true;
        };
        let alone = {
            let mut state = self.lock();
            state.running -= 1;
            state.running == 0
        };
        // Without any listing thread the walk cannot complete; a partial walk
        // must fail, never pass as a snapshot.
        if alone {
            self.fail(io::Error::new(
                error.kind(),
                format!("sync walk worker start failed: {error}"),
            ));
        } else {
            self.changed.notify_all();
        }
        false
    }

    fn worker(&self) {
        let mut idle_since: Option<Instant> = None;
        loop {
            let next = {
                let mut state = self.lock();
                loop {
                    if state.done || state.error.is_some() || self.canceled() {
                        break None;
                    }
                    if let Some(next) = state.queue.pop_front() {
                        state.busy += 1;
                        if self.flow.is_some() {
                            // Counted until its listing permit is granted.
                            state.waiting += 1;
                        }
                        break Some(next);
                    }
                    let waited = idle_since.get_or_insert_with(Instant::now).elapsed();
                    if state.busy == 0 || waited >= WORKER_LINGER {
                        break None;
                    }
                    state = self.wait(state, WORKER_LINGER - waited);
                }
            };
            let Some((dir, depth)) = next else {
                break;
            };
            idle_since = None;
            match self.visit(&dir, depth) {
                Ok(listed) => self.merge(listed, depth),
                Err(error) => self.fail(error),
            }
            self.lock().busy -= 1;
            self.changed.notify_all();
        }
        self.lock().running -= 1;
        self.changed.notify_all();
    }

    /// Lists one folder (under a listing permit on a remote side) and checks
    /// its entries.
    fn visit(&self, dir: &str, depth: usize) -> io::Result<Listed> {
        let permit = match &self.flow {
            Some(flow) => {
                let permit = flow.acquire_meta(self.context.cancel);
                self.lock().waiting -= 1;
                self.changed.notify_all();
                Some(permit.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Interrupted,
                        "synchronization tree walk canceled",
                    )
                })?)
            }
            None => None,
        };
        if depth > MAX_WALK_DEPTH {
            if let Some(permit) = permit {
                permit.abandon();
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("sync tree exceeds {MAX_WALK_DEPTH} levels"),
            ));
        }
        let entries = list_plain_directory(self.context.be, dir);
        finish_listing(permit, entries.as_ref().err());
        scan_listing(self.context, dir, entries?)
    }

    fn merge(&self, listed: Listed, depth: usize) {
        let Listed { mut files, dirs } = listed;
        if !files.is_empty() {
            files.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| right.1.mtime_ms.cmp(&left.1.mtime_ms))
                    .then_with(|| left.2.cmp(&right.2))
            });
            let mut out = self
                .out
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut prior_rel: Option<String> = None;
            for (rel, sig, _) in files {
                if prior_rel.as_deref() != Some(&rel) {
                    out.insert(rel.clone(), sig);
                    prior_rel = Some(rel);
                }
            }
        }
        if !dirs.is_empty() {
            self.lock()
                .queue
                .extend(dirs.into_iter().map(|dir| (dir, depth + 1)));
            self.changed.notify_all();
        }
    }
}

#[cfg(test)]
#[path = "snapshot_walk_tests.rs"]
mod walk_tests;
