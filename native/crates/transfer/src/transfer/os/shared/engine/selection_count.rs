//! The count behind "Zielordner entscheiden": for one selected folder, how
//! many of its files are new at the target and how the same-named ones relate
//! (newer, equal, target newer, conflict; `update_rule`). It never mutates
//! either side.
//!
//! The target is read once per folder with one tolerant listing
//! (`vfs::list_dir_tolerant`), not with a `stat` per file, and the folders are
//! counted in parallel by as many threads as the slower side takes
//! (`Backend::parallelism`). A name the target cannot list is present but
//! unknown: a source file of that name counts as a conflict (both are kept),
//! and the other files of the folder are counted as usual. Only the calling
//! thread reports, at the engine's progress interval.
//!
//! A source folder with the key of the same-named target folder
//! (`update::compare`, keys formed where the files are) is equal as a whole:
//! its target folder is not listed and its files count as equal without a
//! comparison each. When nothing in its subtree needs a look either (no
//! omission, no file of unknown time, `Keys::settled`), the source folder is
//! not listed too: the key walk's tally gives the count, and no sample names
//! come from it.
//!
//! Equal sizes exactly one hour apart where a side stores local time (FAT)
//! count as equal only when both files, hashed where they are stored, hold
//! the same bytes (`time_window::same_content_where_stored`), as the engine
//! decides them.
//!
//! The five counts describe the file rule "neuere ersetzen ältere"; the
//! surfaces show them only for a job that applies the rule
//! (`TransferJob::applies_update_rule`), else the files count as new or
//! same-named like before the rule.
use super::super::lock;
use super::super::update::Keys;
use crate::transfer::engine_names::{join_rel, name_problem};
use crate::transfer::entries::{
    compile_remote_filter, remote_file_entry, RemoteFilterCtx, TransferCollectionBudget,
};
use crate::transfer::time_window::{same_content_where_stored, Stored};
use crate::transfer::update_rule::{classify_local_time, Case, Stamp};
use crate::transfer::{Endpoint, FolderConflictPreview, TransferJob};
use crate::vfs::{Backend, LocalBackend, VfsMeta};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[path = "selection_count_target.rs"]
mod target;
use target::{equal_by_key, equal_keys, read_target, settled_by_key, Target};

/// How long an idle thread waits for the others before it looks again.
const WAIT: Duration = Duration::from_millis(100);
/// Names shown as samples and omitted paths shown.
const SHOWN: usize = 5;

/// A folder still to count: its source path, its path relative to the target
/// folder, and whether the target folder is known not to exist.
struct Node {
    path: String,
    relative: String,
    /// The folder below the selected one (`""` = the selected folder): what
    /// the key comparison names it by.
    inner: String,
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
    local_time: bool,
    /// Folders whose keys are equal at both sides (`update::compare`).
    keys: Option<Keys>,
    cancel: &'a AtomicBool,
    budget: Mutex<TransferCollectionBudget>,
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
    let keys = equal_keys(job, &view, preview, cancel);
    let context = Context {
        source,
        target,
        target_dir: &job.target_dir,
        target_local: matches!(job.target, Endpoint::Local),
        filter: compile_remote_filter(job.filter.clone()),
        allow_backslash: view.allow_backslash(),
        window: super::super::update::window_of(&view),
        local_time: super::super::update::local_time_of(&view),
        keys,
        cancel,
        budget: Mutex::new(TransferCollectionBudget::default()),
    };
    let threads = source.parallelism().min(target.parallelism()).max(1);
    let shared = Shared {
        state: Mutex::new(Counting {
            stack: vec![Node {
                path: preview.source.clone(),
                relative: preview.relative.clone(),
                inner: String::new(),
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

/// Counts the files of one source folder and finds its subfolders.
fn count_folder(context: &Context<'_>, node: &Node) -> Found {
    let mut found = Found::default();
    if let Some(tally) = settled_by_key(context, node) {
        found.equal = tally.files;
        // Its file names are not read (that is the point of the key): the
        // folder itself stands for its same-named files among the samples.
        if tally.files > 0 && !node.relative.is_empty() {
            found.samples.push(format!("{}/", node.relative));
        }
        return found;
    }
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
    let target = if equal_by_key(context, node, &listed.entries) {
        Target::Equal
    } else {
        read_target(context, node)
    };
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
        Target::Unreadable | Target::Equal => false,
        // A name the target cannot list may be the folder: not known absent.
        Target::Listed { .. } => {
            !target.omits(&entry.name)
                && !matches!(
                    target.find(&entry.name),
                    Some([existing]) if existing.is_dir && !existing.is_symlink
                )
        }
    };
    Node {
        inner: if node.inner.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", node.inner, entry.name)
        },
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
    if matches!(target, Target::Equal) {
        // The key of the folder is the key of its source folder: the file is
        // there, with its size and its time.
        found.equal += 1;
        if found.samples.len() < SHOWN {
            found.samples.push(relative);
        }
        return;
    }
    let omitted = target.omits(&entry.name);
    let existing: &[VfsMeta] = match target.find(&entry.name) {
        Some(existing) => existing,
        // Present but unknown: not new, not comparable; both are kept.
        None if omitted => &[],
        None => {
            found.new += 1;
            return;
        }
    };
    let case = match existing {
        [one] if !omitted && !one.is_dir && !one.is_symlink && !one.special => classify_local_time(
            Stamp {
                size: entry.size,
                mtime_ms: entry.mtime_ms,
            },
            Stamp {
                size: one.size,
                mtime_ms: one.mtime_ms,
            },
            context.window,
            context.local_time,
            || hour_proof(context, source, &relative, &one.name),
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

/// The proof the FAT hour needs, as the engine asks it (`update.rs`): both
/// files hashed where they are stored; `None` where a side cannot.
fn hour_proof(context: &Context<'_>, source: &str, relative: &str, name: &str) -> Option<bool> {
    let parent = relative.rsplit_once('/').map_or("", |(parent, _)| parent);
    let target = join_rel(&join_rel(context.target_dir, parent), name);
    same_content_where_stored(
        Stored::new(context.source, source),
        Stored::new(context.target, &target),
        context.cancel,
    )
}

#[cfg(test)]
#[path = "selection_count_tests.rs"]
mod tests;
