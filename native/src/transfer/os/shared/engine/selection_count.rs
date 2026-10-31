//! The count behind "Zielordner entscheiden": for one selected folder, how
//! many of its files are new at the target and how the same-named ones relate
//! (newer, equal, target newer, conflict; `update_rule`). It never mutates
//! either side.
//!
//! The target is read once per folder with one listing, not with a `stat` per
//! file, and the folders are counted in parallel by as many threads as the
//! slower side takes (`Backend::parallelism`). Only the calling thread
//! reports, at the engine's progress interval.
use super::super::lock;
use crate::transfer::engine_names::{join_rel, name_problem};
use crate::transfer::entries::{
    compile_remote_filter, remote_file_entry, RemoteFilterCtx, TransferCollectionBudget,
};
use crate::transfer::update_rule::{classify, Case, Stamp};
use crate::transfer::{Endpoint, FolderConflictPreview, TransferJob};
use crate::vfs::{Backend, LocalBackend, VfsMeta};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// How long an idle thread waits for the others before it looks again.
const WAIT: Duration = Duration::from_millis(100);
/// Names shown as samples and omitted paths shown.
const SHOWN: usize = 5;

/// A folder still to count: its source path, its path relative to the target
/// folder, and whether the target folder is known not to exist.
struct Node {
    path: String,
    relative: String,
    depth: usize,
    target_absent: bool,
}

/// What counting one folder found.
#[derive(Default)]
struct Found {
    new: u64,
    newer: u64,
    equal: u64,
    target_newer: u64,
    conflict: u64,
    samples: Vec<String>,
    omitted: Vec<String>,
    incomplete: bool,
    /// The collection budget was exceeded: the count ends here.
    abort: Option<String>,
    children: Vec<Node>,
}

struct Counting {
    stack: Vec<Node>,
    active: usize,
    preview: FolderConflictPreview,
    complete: bool,
    ended: bool,
}

struct Shared {
    state: Mutex<Counting>,
    wake: Condvar,
}

/// What every thread reads.
struct Context<'a> {
    source: &'a dyn Backend,
    target: &'a dyn Backend,
    target_dir: &'a str,
    target_local: bool,
    filter: Option<RemoteFilterCtx>,
    allow_backslash: bool,
    window: u64,
    cancel: &'a AtomicBool,
    budget: Mutex<TransferCollectionBudget>,
}

/// The names of one target folder.
enum Target {
    Absent,
    Unreadable,
    Listed {
        folded: bool,
        names: HashMap<String, Vec<VfsMeta>>,
    },
}

impl Target {
    fn find(&self, name: &str) -> Option<&[VfsMeta]> {
        let Self::Listed { folded, names } = self else {
            return None;
        };
        let key = if *folded {
            name.to_lowercase()
        } else {
            name.to_string()
        };
        names.get(&key).map(Vec::as_slice)
    }
}

pub fn count_folder_conflict(
    job: &TransferJob,
    preview: &mut FolderConflictPreview,
    cancel: &AtomicBool,
    report: &dyn Fn(&FolderConflictPreview),
) {
    let local = LocalBackend::new("/");
    let source = super::backend(&job.source, &local);
    let target = super::backend(&job.target, &local);
    let view = super::super::JobView::of(job);
    let context = Context {
        source,
        target,
        target_dir: &job.target_dir,
        target_local: matches!(job.target, Endpoint::Local),
        filter: compile_remote_filter(job.filter.clone()),
        allow_backslash: view.allow_backslash(),
        window: super::super::update::window_of(&view),
        cancel,
        budget: Mutex::new(TransferCollectionBudget::default()),
    };
    let threads = source.parallelism().min(target.parallelism()).max(1);
    let shared = Shared {
        state: Mutex::new(Counting {
            stack: vec![Node {
                path: preview.source.clone(),
                relative: preview.relative.clone(),
                depth: 0,
                target_absent: !preview.existing,
            }],
            active: 0,
            preview: preview.clone(),
            complete: true,
            ended: false,
        }),
        wake: Condvar::new(),
    };
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| work(&context, &shared));
        }
        loop {
            let state = lock(&shared.state);
            if state.ended || cancel.load(Ordering::Acquire) {
                break;
            }
            let (state, _) = shared
                .wake
                .wait_timeout(state, super::super::PROGRESS_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.ended {
                break;
            }
            let snapshot = state.preview.clone();
            drop(state);
            report(&snapshot);
        }
    });
    if cancel.load(Ordering::Acquire) {
        return;
    }
    let counting = shared
        .state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *preview = counting.preview;
    preview.complete = counting.complete;
    preview.counted = true;
    report(preview);
}

/// One thread: takes a folder, counts it without any lock, merges the result.
fn work(context: &Context<'_>, shared: &Shared) {
    loop {
        let node = {
            let mut state = lock(&shared.state);
            loop {
                if state.ended || context.cancel.load(Ordering::Acquire) {
                    return;
                }
                if let Some(node) = state.stack.pop() {
                    state.active += 1;
                    break node;
                }
                if state.active == 0 {
                    state.ended = true;
                    shared.wake.notify_all();
                    return;
                }
                state = shared
                    .wake
                    .wait_timeout(state, WAIT)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
        };
        let found =
            crate::source_media::with_cancellation(context.cancel, || count_folder(context, &node));
        let mut state = lock(&shared.state);
        state.active -= 1;
        merge(&mut state, found);
        shared.wake.notify_all();
    }
}

fn merge(state: &mut Counting, found: Found) {
    let preview = &mut state.preview;
    preview.new_files += found.new;
    preview.newer_files += found.newer;
    preview.equal_files += found.equal;
    preview.target_newer_files += found.target_newer;
    preview.conflict_files += found.conflict;
    preview.colliding_files += found.newer + found.equal + found.target_newer + found.conflict;
    for sample in found.samples {
        if preview.samples.len() < SHOWN {
            preview.samples.push(sample);
        }
    }
    for path in found.omitted {
        preview.omitted += 1;
        if preview.omitted_paths.len() < SHOWN {
            preview.omitted_paths.push(path);
        }
    }
    state.complete &= !found.incomplete;
    if let Some(note) = found.abort {
        state.preview.note = Some(note);
        state.complete = false;
        state.ended = true;
        return;
    }
    state.stack.extend(found.children);
}

/// The target folder of `node`: one listing.
fn read_target(context: &Context<'_>, node: &Node) -> Target {
    if node.target_absent {
        return Target::Absent;
    }
    let dir = join_rel(context.target_dir, &node.relative);
    match context.target.list_dir(&dir) {
        Ok(entries) => {
            let folded = if context.target_local {
                cfg!(windows)
            } else {
                !context.target.case_sensitive_paths(&dir)
            };
            let mut names: HashMap<String, Vec<VfsMeta>> = HashMap::with_capacity(entries.len());
            for entry in entries {
                let key = if folded {
                    entry.name.to_lowercase()
                } else {
                    entry.name.clone()
                };
                names.entry(key).or_default().push(entry);
            }
            Target::Listed { folded, names }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Target::Absent,
        Err(_) => Target::Unreadable,
    }
}

/// Counts the files of one source folder and finds its subfolders.
fn count_folder(context: &Context<'_>, node: &Node) -> Found {
    let mut found = Found::default();
    let listed = match crate::vfs::list_dir_tolerant(context.source, &node.path) {
        Ok(listed) => listed,
        Err(_) => {
            found.omitted.push(node.path.clone());
            found.incomplete = true;
            return found;
        }
    };
    found.incomplete |= !listed.omitted.is_empty();
    for omission in listed.omitted {
        found.omitted.push(join_rel(&node.path, &omission.rel));
    }
    let target = read_target(context, node);
    for entry in listed.entries {
        if context.cancel.load(Ordering::Acquire) {
            return found;
        }
        if name_problem(&entry.name, context.allow_backslash).is_some() {
            found.omitted.push(join_rel(&node.path, &entry.name));
            found.incomplete = true;
            continue;
        }
        let child_source = join_rel(&node.path, &entry.name);
        let child_relative = join_rel(&node.relative, &entry.name);
        let recorded = lock(&context.budget).record_node(
            node.depth + 1,
            &[&child_source, &child_relative, &entry.name],
        );
        if let Err(error) = recorded {
            found.abort = Some(error);
            return found;
        }
        if entry.is_symlink || entry.special || crate::apptrash::excluded_name(&entry.name) {
            found.omitted.push(child_source);
            found.incomplete = true;
            continue;
        }
        if let Some(filter) = &context.filter {
            let row = remote_file_entry(
                &child_source,
                &node.path,
                &entry,
                filter.depth_for(&child_source),
            );
            if entry.is_dir {
                if filter.allows_dir_descendants(&row) {
                    found
                        .children
                        .push(child(node, &target, child_source, child_relative, &entry));
                }
                continue;
            }
            if !filter.matches(&row) {
                continue;
            }
        }
        if entry.is_dir {
            found
                .children
                .push(child(node, &target, child_source, child_relative, &entry));
            continue;
        }
        count_file(
            context,
            &target,
            &entry,
            &child_source,
            child_relative,
            &mut found,
        );
    }
    found
}

/// A subfolder: its target folder is absent when the target folder lists no
/// folder of that name (or none at all).
fn child(node: &Node, target: &Target, path: String, relative: String, entry: &VfsMeta) -> Node {
    let target_absent = match target {
        Target::Absent => true,
        Target::Unreadable => false,
        Target::Listed { .. } => !matches!(
            target.find(&entry.name),
            Some([existing]) if existing.is_dir && !existing.is_symlink
        ),
    };
    Node {
        path,
        relative,
        depth: node.depth + 1,
        target_absent,
    }
}

/// One source file against the target folder's listing.
fn count_file(
    context: &Context<'_>,
    target: &Target,
    entry: &VfsMeta,
    source: &str,
    relative: String,
    found: &mut Found,
) {
    if matches!(target, Target::Unreadable) {
        found.omitted.push(source.to_string());
        found.incomplete = true;
        return;
    }
    let Some(existing) = target.find(&entry.name) else {
        found.new += 1;
        return;
    };
    let case = match existing {
        [one] if !one.is_dir && !one.is_symlink && !one.special => classify(
            Stamp {
                size: entry.size,
                mtime_ms: entry.mtime_ms,
            },
            Stamp {
                size: one.size,
                mtime_ms: one.mtime_ms,
            },
            context.window,
        ),
        // A folder, link or several entries under that name: not the same
        // file, both are kept.
        _ => Case::Conflict,
    };
    match case {
        Case::Equal => found.equal += 1,
        Case::SourceNewer => found.newer += 1,
        Case::TargetNewer => found.target_newer += 1,
        Case::Conflict => found.conflict += 1,
    }
    if found.samples.len() < SHOWN {
        found.samples.push(relative);
    }
}

#[cfg(test)]
#[path = "selection_count_tests.rs"]
mod tests;
