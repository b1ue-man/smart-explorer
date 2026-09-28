//! Streaming, parallel discovery of a selection. Folders are listed
//! concurrently under the source connection's flow, and every entry is
//! emitted as soon as it is known, a folder always before its contents.
//! Nothing is collected first: a transfer starts with the first listing, and
//! an Explorer hand-off lists exactly once, when the Explorer asks.
use super::entries::{remote_file_entry, remote_parent, validate_transfer_name, RemoteFilterCtx};
use super::flow::{classify_error, Flow};
use super::flow_control::OpOutcome;
use super::walk_listers::{Listed, Lister};
use crate::types::FileEntry;
use std::collections::{HashSet, VecDeque};
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
    },
    /// Left out on purpose (the active app trash); not an error.
    Omitted { path: String },
    /// Not transferred: listing failed, link or special file, invalid or
    /// duplicate name, nesting too deep.
    Problem { path: String, message: String },
}

pub(crate) struct WalkOptions {
    pub filter: Option<RemoteFilterCtx>,
    /// Emit folder events; unfiltered tree copies keep empty folders.
    pub folders: bool,
    /// Every file lands directly in the target folder under its own name.
    pub flatten: bool,
}

struct Pending {
    path: String,
    rel: String,
    depth: usize,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<Pending>,
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
/// was canceled or stopped before it finished.
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
    for root in roots {
        if walk.stopped() {
            break;
        }
        walk.root(root);
    }
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
        let running = walk.listers.load(Ordering::Acquire);
        let allowed = flow.snapshot().limit.max(1);
        if queued > 0 && running < queued && running < allowed {
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

fn base_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
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

fn outcome<T>(result: &std::io::Result<T>) -> OpOutcome {
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

    fn send(&self, event: WalkEvent) -> bool {
        if (self.emit)(event) {
            return true;
        }
        self.lock().stop = true;
        self.changed.notify_all();
        false
    }

    fn problem(&self, path: &str, message: impl Into<String>) -> bool {
        self.send(WalkEvent::Problem {
            path: path.to_string(),
            message: message.into(),
        })
    }

    fn queue(&self, pending: Pending) {
        self.lock().queue.push_back(pending);
        self.changed.notify_all();
    }

    fn root(&self, root: &WalkRoot) {
        let Some(permit) = self.flow.acquire(self.cancel) else {
            return;
        };
        let stat = self.lister.stat(&root.path);
        permit.finish(outcome(&stat));
        let listed = match stat {
            Ok(listed) => listed,
            Err(error) => {
                self.problem(&root.path, error.to_string());
                return;
            }
        };
        let name = base_name(&root.path);
        if crate::apptrash::excluded_name(name) {
            self.send(WalkEvent::Omitted {
                path: root.path.clone(),
            });
            return;
        }
        if let Some(problem) = listed.problem.as_deref() {
            self.problem(&root.path, problem);
            return;
        }
        if let Err(error) = validate_transfer_name(name, &root.path) {
            self.problem(&root.path, error);
            return;
        }
        if listed.is_link {
            self.problem(&root.path, LINK_REFUSED);
            return;
        }
        if listed.is_dir {
            if self.options.folders
                && !self.options.flatten
                && !self.send(WalkEvent::Dir {
                    path: root.path.clone(),
                    rel: root.rel.clone(),
                })
            {
                return;
            }
            self.queue(Pending {
                path: root.path.clone(),
                rel: root.rel.clone(),
                depth: 0,
            });
            return;
        }
        let rel = if self.options.flatten {
            name.to_string()
        } else {
            root.rel.clone()
        };
        self.send(WalkEvent::File {
            path: root.path.clone(),
            rel,
            size: listed.size,
            mtime_ms: listed.mtime_ms,
            id: listed.id,
        });
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
            let Some(pending) = next else {
                break;
            };
            self.list_one(pending);
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
        let Some(permit) = self.flow.acquire(self.cancel) else {
            return;
        };
        let listed = self.lister.list(&pending.path);
        permit.finish(outcome(&listed));
        let children = match listed {
            Ok(children) => children,
            Err(error) => {
                self.problem(&pending.path, error.to_string());
                return;
            }
        };
        let mut names = HashSet::with_capacity(children.len());
        for child in children {
            if self.stopped() {
                return;
            }
            let path = join(&pending.path, &child.name);
            if !names.insert(child.name.clone()) {
                let message = format!("Backend lieferte den Namen {:?} mehrfach", child.name);
                if !self.problem(&path, message) {
                    return;
                }
                continue;
            }
            if crate::apptrash::excluded_name(&child.name) {
                if !self.send(WalkEvent::Omitted { path }) {
                    return;
                }
                continue;
            }
            let refused = child
                .problem
                .clone()
                .or_else(|| validate_transfer_name(&child.name, &pending.path).err())
                .or_else(|| child.is_link.then(|| LINK_REFUSED.to_string()));
            if let Some(message) = refused {
                if !self.problem(&path, message) {
                    return;
                }
                continue;
            }
            if !self.child(&pending, path, child) {
                return;
            }
        }
    }

    fn child(&self, parent: &Pending, path: String, child: Listed) -> bool {
        let rel = if self.options.flatten {
            child.name.clone()
        } else {
            format!("{}/{}", parent.rel, child.name)
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
            self.queue(Pending {
                path,
                rel,
                depth: parent.depth + 1,
            });
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
        })
    }
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
