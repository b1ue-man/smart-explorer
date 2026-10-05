//! The folders of one snapshot walk, listed by a pool that grows while
//! folders wait: up to the flow's limit plus its reserved listing slot on a
//! remote side (every listing under a listing permit, repeated after
//! overload), up to `parallelism()` threads on a local one. The first error,
//! a cancellation or a worker's panic fails the whole walk: a partial tree
//! never becomes a snapshot.
use crate::transfer::Flow;
use crate::vfs::Backend;
use std::collections::VecDeque;
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::snapshot_dir::{scan_listing, Listed, WalkContext};
use super::sync_overload::under_permits;
use super::types::Tree;

const MAX_WALK_DEPTH: usize = 512;
/// The coordinator looks at the flow at least this often (its limit moves
/// while other jobs use the connection).
const COORDINATOR_SLICE: Duration = Duration::from_millis(50);
/// An idle listing thread waits this long for the folders a busy one is
/// about to find: longer than one listing round trip on common links (the
/// transfer engine's reasoning), short against the walk.
const WORKER_LINGER: Duration = Duration::from_millis(250);

/// Walks `context.root`; `flow` is the remote side's flow (`None` locally).
pub(super) fn walk_tree(
    context: &WalkContext<'_>,
    flow: Option<Arc<Flow>>,
    local_threads: usize,
) -> io::Result<Tree> {
    let walk = TreeWalk {
        context,
        flow,
        local_threads,
        state: Mutex::new(WalkState::default()),
        changed: Condvar::new(),
        out: Mutex::new(Tree::new()),
    };
    walk.lock()
        .queue
        .push_back((context.root.to_string(), String::new(), 0));
    std::thread::scope(|scope| walk.coordinate(scope));

    // A worker can observe cancellation while it is part-way through a
    // directory. Never turn that partial walk into a successful snapshot:
    // callers persist successful walks as the next deletion baseline.
    if context.cancel.load(Ordering::Relaxed) {
        return Err(canceled_error());
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

fn canceled_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "synchronization tree walk canceled",
    )
}

fn list_plain_directory(ctx: &WalkContext<'_>, path: &str) -> io::Result<crate::vfs::VfsListing> {
    let be = ctx.be;
    let metadata = crate::vfs::sync_stat(be, path)?;
    if metadata.is_symlink || !metadata.is_dir || metadata.special {
        return Err(super::apply_boundary::protected(if metadata.is_symlink {
            super::OmissionKind::Link
        } else {
            super::OmissionKind::Special
        }));
    }
    if path != ctx.root {
        if let Some(kind) =
            super::snapshot_policy::protected(be, ctx.root, path, &metadata, ctx.opts.cross_mounts)?
        {
            return Err(super::apply_boundary::protected(kind));
        }
    }
    crate::vfs::list_dir_tolerant(be, path)
}

#[derive(Default)]
struct WalkState {
    /// Folders to list, with their depth below the root.
    queue: VecDeque<(String, String, usize)>,
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

    /// Starts one listing thread; false when the system refused it. A panic
    /// fails the walk through the thread's `Enlisted` guard and is caught
    /// here, so the scope never re-raises it in the caller.
    fn spawn<'s>(&'s self, scope: &'s std::thread::Scope<'s, '_>) -> bool {
        let spawned = std::thread::Builder::new()
            .name("sync-walk".to_string())
            .spawn_scoped(scope, move || {
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| self.worker()));
            });
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
        let mut enlisted = Enlisted {
            walk: self,
            busy: false,
            waiting: false,
        };
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
            let Some((dir, rel, depth)) = next else {
                break;
            };
            enlisted.busy = true;
            enlisted.waiting = self.flow.is_some();
            idle_since = None;
            match self.visit(&dir, &rel, depth, &mut enlisted) {
                Ok(listed) => self.merge(listed, depth),
                Err(error) => self.fail(error),
            }
            self.stop_waiting(&mut enlisted);
            enlisted.busy = false;
            self.lock().busy -= 1;
            self.changed.notify_all();
        }
    }

    fn stop_waiting(&self, enlisted: &mut Enlisted<'_, '_>) {
        if enlisted.waiting {
            enlisted.waiting = false;
            self.lock().waiting -= 1;
            self.changed.notify_all();
        }
    }

    /// Lists one folder (on a remote side under a listing permit, again after
    /// overload) and checks its entries.
    fn visit(
        &self,
        dir: &str,
        dir_rel: &str,
        depth: usize,
        enlisted: &mut Enlisted<'_, '_>,
    ) -> io::Result<Listed> {
        if depth > MAX_WALK_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("sync tree exceeds {MAX_WALK_DEPTH} levels"),
            ));
        }
        let entries = match &self.flow {
            None => list_plain_directory(self.context, dir),
            Some(flow) => under_permits(
                self.context.cancel,
                &self.context.progress,
                || {
                    if !enlisted.waiting {
                        self.lock().waiting += 1;
                        enlisted.waiting = true;
                    }
                    let permit = flow.acquire_meta(self.context.cancel);
                    self.stop_waiting(enlisted);
                    permit
                },
                |_| list_plain_directory(self.context, dir),
            )
            .unwrap_or_else(|| Err(canceled_error())),
        };
        let listing = match entries {
            Ok(listing) => listing,
            Err(error) if dir != self.context.root => {
                if let Some(reason) =
                    super::apply_boundary::omitted(&error).or_else(|| match error.kind() {
                        io::ErrorKind::PermissionDenied => Some(super::OmissionKind::Unreadable),
                        io::ErrorKind::NotFound => Some(super::OmissionKind::Vanished),
                        _ => None,
                    })
                {
                    super::snapshot_dir::record_omission(self.context, dir_rel, reason, true);
                    return Ok(Listed {
                        files: Vec::new(),
                        dirs: Vec::new(),
                    });
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        for omission in listing.omitted {
            let rel = match super::sync_relative_path::validate_component(&omission.rel) {
                Ok(()) => super::snapshot_dir::literal_child(dir_rel, &omission.rel),
                Err(error) if dir_rel.is_empty() => return Err(error),
                Err(_) => dir_rel.to_string(),
            };
            let filtered = self.context.filter.ignored(&rel, true);
            super::snapshot_dir::record_omission(
                self.context,
                &rel,
                omission.reason.into(),
                !filtered,
            );
        }
        scan_listing(self.context, dir, dir_rel, listing.entries)
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
                .extend(dirs.into_iter().map(|(dir, rel)| (dir, rel, depth + 1)));
            self.changed.notify_all();
        }
    }
}

/// A listing thread's place in the walk, given back when it ends. A panic
/// gives it back as well and fails the walk, so the walk neither hangs nor
/// ever passes a partial tree as a snapshot.
struct Enlisted<'w, 'a> {
    walk: &'w TreeWalk<'a>,
    /// Holding a folder.
    busy: bool,
    /// Counted as waiting for a listing permit.
    waiting: bool,
}

impl Drop for Enlisted<'_, '_> {
    fn drop(&mut self) {
        let mut state = self.walk.lock();
        state.running -= 1;
        if self.busy {
            state.busy -= 1;
        }
        if self.waiting {
            state.waiting -= 1;
        }
        if std::thread::panicking() && state.error.is_none() {
            state.error = Some(io::Error::other("a sync walk worker stopped unexpectedly"));
        }
        drop(state);
        self.walk.changed.notify_all();
    }
}
