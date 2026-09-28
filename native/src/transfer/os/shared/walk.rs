//! Streaming, parallel discovery of a selection. Folders are listed
//! concurrently under the source connection's flow (listings may use its
//! reserved metadata slot), and every entry is emitted as soon as it is known,
//! a folder always before its contents. Nothing is collected first: a
//! transfer starts with the first listing, and an Explorer hand-off lists
//! exactly once, when the Explorer asks.
use super::access::{with_access_detail, AccessAnswer, AccessGate};
use super::entries::{remote_file_entry, remote_parent, validate_transfer_name, RemoteFilterCtx};
use super::flow::{classify_error, Flow};
use super::flow_control::OpOutcome;
use super::walk_listers::{Listed, Lister};
use crate::types::FileEntry;
use crate::vfs::remote_util::numbered_remote_name;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

const MAX_WALK_DEPTH: usize = 512;
const LINK_REFUSED: &str = "Links, Reparse-Punkte und Spezialdateien werden nicht übertragen";
const COORDINATOR_SLICE: Duration = Duration::from_millis(50);

/// A selected entry and its path relative to the target folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WalkRoot {
    pub path: String,
    pub rel: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WalkEvent {
    /// A folder to create below the target (only with `folders`).
    Dir { path: String, rel: String },
    File {
        path: String,
        rel: String,
        size: u64,
        mtime_ms: i64,
        id: Option<String>,
        md5: Option<String>,
    },
    /// Left out on purpose (the active app trash); not an error.
    Omitted { path: String },
    /// Not transferred: listing failed, link or special file, invalid or
    /// duplicate name, nesting too deep.
    Problem { path: String, message: String },
    /// The user declined read access for a protected folder; the walk stops.
    AccessRefused { path: String },
}

#[derive(Default)]
pub(crate) struct WalkOptions {
    pub filter: Option<RemoteFilterCtx>,
    /// Emit folder events; unfiltered tree copies keep empty folders.
    pub folders: bool,
    /// Every file lands directly in the target folder under its own name.
    pub flatten: bool,
    /// Asked once when a local listing is refused (protected folders).
    pub access: Option<Arc<AccessGate>>,
    /// Names may contain `\`: local copies on Unix, where it is an ordinary
    /// character. Every other rule of `vfs::validate_child_name` still holds.
    pub allow_backslash: bool,
}

struct Pending {
    path: String,
    rel: String,
    depth: usize,
}

enum Task {
    /// Selected entries of one folder, answered by one listing of it.
    Roots {
        parent: String,
        members: Vec<WalkRoot>,
    },
    /// One selected entry, inspected on its own.
    Root(WalkRoot),
    Dir(Pending),
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<Task>,
    active: usize,
    stop: bool,
}

struct Walk<'a> {
    lister: &'a dyn Lister,
    options: &'a WalkOptions,
    flow: &'a Arc<Flow>,
    cancel: &'a AtomicBool,
    emit: &'a (dyn Fn(WalkEvent) -> bool + Sync),
    shared: Mutex<Shared>,
    changed: Condvar,
    listers: AtomicUsize,
}

/// Walks `roots` and hands every event to `emit`, which may block (bounded
/// channels) and returns false to stop the walk. Returns false when the walk
/// was canceled or stopped (also by a declined access request) before it
/// finished.
pub(crate) fn walk(
    lister: &dyn Lister,
    roots: &[WalkRoot],
    options: &WalkOptions,
    flow: &Arc<Flow>,
    cancel: &AtomicBool,
    emit: &(dyn Fn(WalkEvent) -> bool + Sync),
) -> bool {
    let walk = Walk {
        lister,
        options,
        flow,
        cancel,
        emit,
        shared: Mutex::new(Shared::default()),
        changed: Condvar::new(),
        listers: AtomicUsize::new(0),
    };
    walk.queue_roots(roots);
    std::thread::scope(|scope| loop {
        let mut shared = walk.lock();
        if shared.stop || cancel.load(Ordering::Acquire) {
            shared.stop = true;
            break;
        }
        let queued = shared.queue.len();
        if queued == 0 && shared.active == 0 {
            break;
        }
        // Listers that hold no task pick up queued ones themselves; start
        // another only for tasks beyond them, within the connection's limit.
        let running = walk.listers.load(Ordering::Acquire);
        let idle = running.saturating_sub(shared.active);
        let allowed = flow.snapshot().limit.max(1);
        if queued > idle && running < allowed {
            drop(shared);
            walk.listers.fetch_add(1, Ordering::AcqRel);
            scope.spawn(|| walk.lister_loop());
            continue;
        }
        drop(
            walk.changed
                .wait_timeout(shared, COORDINATOR_SLICE)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0),
        );
    });
    let stopped = walk.lock().stop;
    !stopped && !cancel.load(Ordering::Acquire)
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

fn filter_entry(path: &str, listed: &Listed, filter: &RemoteFilterCtx) -> FileEntry {
    let meta = crate::vfs::VfsMeta {
        name: listed.name.clone(),
        is_dir: listed.is_dir,
        is_symlink: listed.is_link,
        size: listed.size,
        mtime_ms: listed.mtime_ms,
        btime_ms: listed.btime_ms,
        hidden: listed.hidden,
        system: listed.system,
        id: listed.id.clone(),
        content_md5: None,
    };
    remote_file_entry(path, &remote_parent(path), &meta, filter.depth_for(path))
}

/// Providers such as Drive allow several files of one name in a folder. A
/// further file whose id differs from the first one is still transferred,
/// read by its id, under the first free numbered name. Folders of one name
/// cannot be listed apart and stay a reported problem.
fn duplicate_name(
    child: &Listed,
    first_id: Option<&str>,
    taken: &HashSet<String>,
    assigned: &HashSet<String>,
) -> Option<String> {
    let distinct =
        matches!((first_id, child.id.as_deref()), (Some(first), Some(id)) if first != id);
    if !distinct || child.is_dir || child.is_link {
        return None;
    }
    (2..)
        .map(|index| numbered_remote_name(&child.name, index))
        .find(|name| !taken.contains(name) && !assigned.contains(name))
}

fn outcome<T>(result: &io::Result<T>) -> OpOutcome {
    match result {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    }
}

impl Walk<'_> {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Acquire) || self.lock().stop
    }

    fn halt(&self) {
        self.lock().stop = true;
        self.changed.notify_all();
    }

    fn send(&self, event: WalkEvent) -> bool {
        if (self.emit)(event) {
            return true;
        }
        self.halt();
        false
    }

    fn problem(&self, path: &str, message: impl Into<String>) -> bool {
        self.send(WalkEvent::Problem {
            path: path.to_string(),
            message: message.into(),
        })
    }

    fn queue(&self, task: Task) {
        self.lock().queue.push_back(task);
        self.changed.notify_all();
    }

    /// One listing operation under a metadata permit. A local refusal asks
    /// the access gate outside of every permit and retries once; `None` means
    /// the walk stops (canceled, or the user declined access).
    fn listed<T>(
        &self,
        path: &str,
        operation: impl Fn() -> io::Result<T>,
    ) -> Option<io::Result<T>> {
        let permit = self.flow.acquire_meta(self.cancel)?;
        let result = operation();
        permit.finish(outcome(&result));
        let error = match result {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => error,
            other => return Some(other),
        };
        let Some(gate) = self.options.access.as_ref() else {
            return Some(Err(error));
        };
        match gate.request() {
            AccessAnswer::Granted => {
                let permit = self.flow.acquire_meta(self.cancel)?;
                let retried = operation();
                permit.finish(outcome(&retried));
                Some(retried)
            }
            AccessAnswer::Refused => {
                self.send(WalkEvent::AccessRefused {
                    path: path.to_string(),
                });
                self.halt();
                None
            }
            AccessAnswer::Unavailable(detail) => {
                Some(Err(with_access_detail(error, detail.as_deref())))
            }
        }
    }

    fn name_problem(&self, name: &str, context: &str) -> Option<String> {
        if !self.options.allow_backslash {
            return validate_transfer_name(name, context).err();
        }
        let unsafe_name =
            name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\0']);
        unsafe_name.then(|| format!("{context}: backend returned unsafe child name: {name:?}"))
    }

    /// Link, special file, unaddressable or invalid name: the reason the
    /// entry called `name` is not transferred.
    fn refusal(&self, listed: &Listed, name: &str, context: &str) -> Option<String> {
        listed
            .problem
            .clone()
            .or_else(|| self.name_problem(name, context))
            .or_else(|| listed.is_link.then(|| LINK_REFUSED.to_string()))
    }

    fn lister_loop(&self) {
        loop {
            let next = {
                let mut shared = self.lock();
                if shared.stop || self.cancel.load(Ordering::Acquire) {
                    None
                } else {
                    let next = shared.queue.pop_front();
                    if next.is_some() {
                        shared.active += 1;
                    }
                    next
                }
            };
            let Some(task) = next else {
                break;
            };
            match task {
                Task::Roots { parent, members } => self.roots(&parent, members),
                Task::Root(root) => self.stat_root(root),
                Task::Dir(pending) => self.list_one(pending),
            }
            self.lock().active -= 1;
            self.changed.notify_all();
        }
        self.listers.fetch_sub(1, Ordering::AcqRel);
        self.changed.notify_all();
    }

    fn list_one(&self, pending: Pending) {
        if pending.depth >= MAX_WALK_DEPTH {
            self.problem(
                &pending.path,
                format!("Maximale Verzeichnistiefe von {MAX_WALK_DEPTH} überschritten"),
            );
            return;
        }
        let children = match self.listed(&pending.path, || self.lister.list(&pending.path)) {
            None => return,
            Some(Ok(children)) => children,
            Some(Err(error)) => {
                self.problem(&pending.path, error.to_string());
                return;
            }
        };
        let taken: HashSet<String> = children.iter().map(|child| child.name.clone()).collect();
        let mut first_ids: HashMap<String, Option<String>> = HashMap::with_capacity(children.len());
        let mut assigned: HashSet<String> = HashSet::new();
        for child in children {
            if self.stopped() {
                return;
            }
            let path = join(&pending.path, &child.name);
            let name = match first_ids.get(&child.name) {
                None => {
                    first_ids.insert(child.name.clone(), child.id.clone());
                    child.name.clone()
                }
                Some(first) => match duplicate_name(&child, first.as_deref(), &taken, &assigned) {
                    Some(name) => {
                        assigned.insert(name.clone());
                        name
                    }
                    None => {
                        let message =
                            format!("Backend lieferte den Namen {:?} mehrfach", child.name);
                        if !self.problem(&path, message) {
                            return;
                        }
                        continue;
                    }
                },
            };
            if crate::apptrash::excluded_name(&child.name) {
                if !self.send(WalkEvent::Omitted { path }) {
                    return;
                }
                continue;
            }
            if let Some(message) = self.refusal(&child, &child.name, &pending.path) {
                if !self.problem(&path, message) {
                    return;
                }
                continue;
            }
            if !self.child(&pending, path, &name, child) {
                return;
            }
        }
    }

    /// `name` is the entry's name below the target: its own, or a numbered
    /// one for a second file of the same name (see `duplicate_name`).
    fn child(&self, parent: &Pending, path: String, name: &str, child: Listed) -> bool {
        let rel = if self.options.flatten {
            name.to_string()
        } else {
            format!("{}/{}", parent.rel, name)
        };
        if child.is_dir {
            if let Some(filter) = &self.options.filter {
                if !filter.allows_dir_descendants(&filter_entry(&path, &child, filter)) {
                    return true;
                }
            }
            if self.options.folders
                && !self.options.flatten
                && !self.send(WalkEvent::Dir {
                    path: path.clone(),
                    rel: rel.clone(),
                })
            {
                return false;
            }
            self.queue(Task::Dir(Pending {
                path,
                rel,
                depth: parent.depth + 1,
            }));
            return true;
        }
        if let Some(filter) = &self.options.filter {
            if !filter.matches(&filter_entry(&path, &child, filter)) {
                return true;
            }
        }
        self.send(WalkEvent::File {
            path,
            rel,
            size: child.size,
            mtime_ms: child.mtime_ms,
            id: child.id,
            md5: child.md5,
        })
    }
}

#[path = "walk_roots.rs"]
mod roots;

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
