//! `se cp`, `se get`, `se put` and the MCP `copy` on the transfer engine
//! (flows, packages, ranges, time accounts) for the plain case: a source that
//! is no link, copied to a destination that does not exist yet below an
//! existing folder. Everything else (an existing destination, `--force`, a
//! link as source root, a folder without `--recursive`, `se mv`) stays on the
//! older path in `ops`/`tree_ops`, which keeps its checks and its messages.
//!
//! The terminal keeps its promises on this path: the destination carries
//! exactly the name that was typed, nothing is replaced (a race ends as an
//! error), links and special files inside a tree are reported as omissions,
//! and cancellation stops the run. One difference is deliberate: the engine
//! does not stop at the first failing file (see `docs/GOTCHAS.md`).
#[path = "engine_copy_issues.rs"]
mod issues;
#[path = "engine_copy_stats.rs"]
mod stats;

use super::entry_kind::EntryKind;
use super::ops::{join, parent_of};
use super::target::Target;
use super::tree_guard::{validate_listed_names, validate_same_source};
use super::tree_observer::{TreeObserver, TreePhase, TRANSFER_CANCELLED};
use super::tree_ops::{Omission, TreeOutcome};
use crate::transfer::engine_names::{base_name, validate_pair_rels};
use crate::transfer::walk::{omission_kind, OmissionKind};
use crate::transfer::{
    path_within, JobItems, JobView, Layout, PairItem, ResolvedRoot, Side, TransferIssue,
    TransferMsg, TransferProgress,
};
use crate::types::{Conflict, CopyMode};
use crate::vfs::VfsMeta;
use std::io;
use std::sync::Mutex;

use issues::all_issues;
#[cfg(test)]
pub(super) use issues::parse_log;
use stats::average_rate;
pub(super) use stats::print_stats;
#[cfg(test)]
pub(super) use stats::stats_line;

/// Switch of the engine path (security answer E13-S8 is in place).
pub(super) const CLI_ENGINE_PATH: bool = true;

#[cfg(test)]
thread_local! {
    static LEGACY_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests of the older path's own checks run the closure with the engine path
/// off (this thread only).
#[cfg(test)]
pub(super) fn with_legacy_path<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            LEGACY_ONLY.with(|flag| flag.set(self.0));
        }
    }
    let _reset = Reset(LEGACY_ONLY.with(|flag| flag.replace(true)));
    run()
}

#[cfg(test)]
pub(super) fn enabled_now() -> bool {
    CLI_ENGINE_PATH && !LEGACY_ONLY.with(|flag| flag.get())
}

#[cfg(not(test))]
pub(super) fn enabled_now() -> bool {
    CLI_ENGINE_PATH
}

/// Failing files listed in the error text before it points at the log.
const SHOWN_FAILURES: usize = 5;

/// What decides whether a copy may run on the engine.
pub(super) struct Eligibility {
    pub(super) enabled: bool,
    pub(super) source: EntryKind,
    pub(super) recursive: bool,
    /// `lstat` of the destination said "not found" (a dangling link is there).
    pub(super) destination_absent: bool,
    /// The parent of the destination is an existing folder, not a link.
    pub(super) parent_is_plain_dir: bool,
    /// The destination is not the source and does not lie inside it.
    pub(super) disjoint: bool,
    /// The destination name is one the engine creates as typed.
    pub(super) name_ok: bool,
    /// The engine would ask the user for read access to a protected local
    /// source (`AccessGate`); a terminal or the MCP service must not ask.
    pub(super) access_prompt_possible: bool,
}

/// The pure rule of the engine path (U49-2).
pub(super) fn eligible(facts: &Eligibility) -> bool {
    facts.enabled
        && facts.destination_absent
        && facts.parent_is_plain_dir
        && facts.disjoint
        && facts.name_ok
        && !facts.access_prompt_possible
        && match facts.source {
            EntryKind::File => true,
            EntryKind::Dir => facts.recursive,
            EntryKind::Link | EntryKind::Special => false,
        }
}

/// What the engine reported at the end of a run.
pub(super) struct Finished {
    pub(super) progress: TransferProgress,
    pub(super) canceled: bool,
    pub(super) issues: Vec<TransferIssue>,
    pub(super) roots: Vec<ResolvedRoot>,
}

/// The engine path with the switch `CLI_ENGINE_PATH`.
pub(super) fn try_copy(
    src: &Target,
    src_meta: &VfsMeta,
    source_snapshot: &VfsMeta,
    dst: &Target,
    dst_path: &str,
    recursive: bool,
    observer: &TreeObserver,
) -> Result<Option<TreeOutcome>, String> {
    try_copy_with(
        enabled_now(),
        src,
        src_meta,
        source_snapshot,
        dst,
        dst_path,
        recursive,
        observer,
    )
}

/// `Ok(None)`: not eligible, nothing was touched, the older path decides.
/// Once the check "destination missing" passed, a destination that appears
/// meanwhile is an error, never a merge or a replacement.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_copy_with(
    enabled: bool,
    src: &Target,
    src_meta: &VfsMeta,
    source_snapshot: &VfsMeta,
    dst: &Target,
    dst_path: &str,
    recursive: bool,
    observer: &TreeObserver,
) -> Result<Option<TreeOutcome>, String> {
    if !enabled {
        return Ok(None);
    }
    let Some(parent) = parent_of(dst_path) else {
        return Ok(None);
    };
    if !eligible(&probe(src, src_meta, dst, dst_path, &parent, recursive)) {
        return Ok(None);
    }
    // The source is the one that was planned (same check as the older path).
    let fresh = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    validate_same_source(source_snapshot, &fresh, &src.path)?;
    let (items, target_dir) = if fresh.is_dir {
        let Some(children) = child_paths(src, dst_path)? else {
            return Ok(None);
        };
        match crate::vfs::create_dir_exclusive(&*dst.backend, dst_path) {
            Ok(()) => {}
            // Nothing was created: the older path can still decide.
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
            // A race ends as an error, never as a merge (E13-S8).
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(format!(
                    "destination changed during copy: {dst_path} appeared at the destination"
                ));
            }
            Err(error) => {
                return Err(format!("cannot create directory {dst_path}: {error}"));
            }
        }
        let items = JobItems::Roots {
            paths: children,
            base: Some(src.path.clone()),
        };
        (items, dst_path.to_string())
    } else {
        let name = last_name(dst_path).to_string();
        let pair = PairItem {
            source: src.path.clone(),
            rel: name,
            size: Some(fresh.size),
            mtime_ms: fresh.mtime_ms,
            id: fresh.id.clone(),
        };
        (JobItems::Pairs(vec![pair]), parent)
    };
    observer.begin_engine();
    observer.set_phase(TreePhase::Writing);
    let outcome = if items.is_empty() {
        // An empty folder: the exclusive creation above was the whole copy.
        TreeOutcome::complete(dst_path.to_string())
    } else {
        let finished = run(src, dst, &target_dir, &items, observer)?;
        settle(&finished, dst_path)?
    };
    observer.set_phase(TreePhase::Done);
    Ok(Some(outcome))
}

fn probe(
    src: &Target,
    src_meta: &VfsMeta,
    dst: &Target,
    dst_path: &str,
    parent: &str,
    recursive: bool,
) -> Eligibility {
    // `try_exists` is `stat`, and every backend reports a link as a link
    // (`lstat`): a dangling link is "present", never "missing".
    let destination_absent = matches!(dst.backend.try_exists(dst_path), Ok(false));
    let parent_is_plain_dir = dst
        .backend
        .stat(parent)
        .is_ok_and(|meta| meta.is_dir && !meta.is_symlink);
    let disjoint = !src.same_namespace(dst)
        || !path_within(
            dst_path,
            &src.path,
            dst.backend.case_sensitive_paths(parent),
        );
    Eligibility {
        enabled: true,
        source: EntryKind::of(src_meta),
        recursive,
        destination_absent,
        parent_is_plain_dir,
        disjoint,
        name_ok: validate_pair_rels([last_name(dst_path)], false).is_ok(),
        access_prompt_possible: src.backend.is_local() && access_prompt_possible(src, src_meta),
    }
}

/// Whether the engine's read-access request could be asked for this source:
/// the same folder and the same test as `AccessGate::request`. Where it is
/// true (Windows, local drive, no grant, no backup privilege) the older path
/// reports a refused read as an error instead.
pub(super) fn access_prompt_possible(src: &Target, src_meta: &VfsMeta) -> bool {
    let root = if src_meta.is_dir {
        Some(src.path.clone())
    } else {
        parent_of(&src.path)
    };
    let Some(root) = root else {
        return false;
    };
    let native = std::path::PathBuf::from(root.replace('/', std::path::MAIN_SEPARATOR_STR));
    crate::local_access::can_request_access(&crate::local_access::display_path(&native))
}

fn last_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// The entries of the source folder, each a root of the job. The folder that
/// is about to be created is never one of them. `None` when a name in the
/// listing is unsafe or listed twice: the older path checks the whole tree
/// before it writes anything and reports that case with its own message (the
/// engine refuses such entries too, but only after it started writing).
fn child_paths(src: &Target, dst_path: &str) -> Result<Option<Vec<String>>, String> {
    let listed = src
        .backend
        .list_dir(&src.path)
        .map_err(|error| format!("cannot list {}: {error}", src.path))?;
    if validate_listed_names(&src.path, &listed).is_err() {
        return Ok(None);
    }
    Ok(Some(
        listed
            .into_iter()
            .map(|child| join(&src.path, &child.name))
            .filter(|path| path != dst_path)
            .collect(),
    ))
}

fn side_of(target: &Target) -> Side<'_> {
    if target.backend.is_local() {
        Side::Local
    } else {
        Side::Remote(&*target.backend)
    }
}

/// Runs the job synchronously; the observer follows every progress message.
fn run(
    src: &Target,
    dst: &Target,
    target_dir: &str,
    items: &JobItems,
    observer: &TreeObserver,
) -> Result<Finished, String> {
    let options = RunOptions {
        conflict: Conflict::Skip,
        mode: CopyMode::Copy,
        job_id: None,
    };
    run_job(src, dst, target_dir, items, observer, &options)
}

/// How an engine run of the terminal treats names and sources.
pub(super) struct RunOptions {
    /// `Skip`: a destination that appeared is left alone and reported;
    /// `Overwrite` only for `--force`.
    pub(super) conflict: Conflict,
    pub(super) mode: CopyMode,
    /// A crash log under this job id (moves: an interrupted move resumes
    /// from the recovery catalog instead of leaving a quarantined source).
    pub(super) job_id: Option<String>,
}

/// `run` with the conflict policy, mode and crash log of `options`.
pub(super) fn run_job(
    src: &Target,
    dst: &Target,
    target_dir: &str,
    items: &JobItems,
    observer: &TreeObserver,
    options: &RunOptions,
) -> Result<Finished, String> {
    let view = JobView {
        recoverable: options.job_id.is_some(),
        recovery: crate::transfer::RecoveryMode::New,
        id: options.job_id.as_deref(),
        folders: &crate::transfer::FolderConflictPolicy::new(
            crate::transfer::FolderConflict::Merge,
        ),
        source: side_of(src),
        target: side_of(dst),
        target_dir,
        items,
        layout: Layout::Tree,
        filter: None,
        conflict: options.conflict,
        mode: options.mode,
        source_label: &src.path,
        target_label: target_dir,
        resume: None,
    };
    let finished: Mutex<Option<Finished>> = Mutex::new(None);
    let report = |message: TransferMsg| match message {
        TransferMsg::Progress(progress) => publish(observer, &progress),
        TransferMsg::Done {
            progress,
            canceled,
            issues,
            roots,
            ..
        } => {
            publish(observer, &progress);
            let mut slot = finished
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(Finished {
                progress,
                canceled,
                issues,
                roots,
            });
        }
    };
    crate::transfer::run_view(view, &report, observer.cancel_flag());
    finished
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .ok_or_else(|| "the transfer engine ended without a result".to_string())
}

/// The engine's counters and verdict into the observer. The final message
/// carries the whole run's average rate and the last confirmed verdict.
fn publish(observer: &TreeObserver, progress: &TransferProgress) {
    let rate = if progress.done {
        average_rate(progress.bytes_done, progress.elapsed_ms)
    } else {
        progress.rate_bps
    };
    observer.set_engine_totals(progress.files_total, 0, progress.bytes_total);
    observer.set_engine_done(progress.files_done, progress.bytes_done);
    observer.set_engine_elapsed(progress.elapsed_ms);
    observer.set_engine_stats(
        rate,
        progress.limiter.code(),
        progress.limiter_text.clone(),
        progress.segments,
    );
    observer.set_engine_note(progress.note.clone());
}

/// The result of a finished run in the terms of `se cp` (U49-4 point 5).
pub(super) fn settle(finished: &Finished, dst_path: &str) -> Result<TreeOutcome, String> {
    if finished.canceled {
        return Err(TRANSFER_CANCELLED.to_string());
    }
    let skipped = finished.progress.skipped;
    if skipped > 0 {
        return Err(format!(
            "destination changed during copy: {skipped} existing files were left alone"
        ));
    }
    let issues = all_issues(finished);
    let mut omitted = Vec::new();
    let mut failures: Vec<&TransferIssue> = Vec::new();
    for issue in &issues.items {
        let kind = match omission_kind(&issue.message) {
            Some(OmissionKind::Link) => Some(EntryKind::Link),
            Some(OmissionKind::Special) => Some(EntryKind::Special),
            Some(OmissionKind::Other) | None => None,
        };
        match kind {
            Some(kind) if !issue.path.is_empty() => omitted.push(Omission {
                path: issue.path.clone(),
                kind,
            }),
            _ => failures.push(issue),
        }
    }
    let failed = failures.len() as u64 + issues.unseen;
    if failed > 0 {
        return Err(failure_text(
            failed,
            &failures,
            finished.progress.log_path.as_deref(),
        ));
    }
    if let Some(root) = finished
        .roots
        .iter()
        .find(|root| root.rel != base_name(&root.source))
    {
        // The engine numbers a name that appeared at a remote destination
        // after the check; nothing was replaced, but the name is not the typed one.
        return Err(format!(
            "destination changed during copy: {} appeared at the destination, the data was stored as {} instead",
            base_name(&root.source),
            root.rel
        ));
    }
    Ok(TreeOutcome {
        destination: dst_path.to_string(),
        omitted,
        kept: Vec::new(),
    })
}

fn failure_text(failed: u64, shown: &[&TransferIssue], log: Option<&str>) -> String {
    let mut text = format!("{failed} files failed");
    for issue in shown.iter().take(SHOWN_FAILURES) {
        text.push('\n');
        if issue.path.is_empty() {
            text.push_str(&issue.message);
        } else {
            text.push_str(&format!("{}: {}", issue.path, issue.message));
        }
    }
    if failed > SHOWN_FAILURES as u64 {
        if let Some(log) = log {
            text.push_str(&format!("\nall issues: {log}"));
        }
    }
    text
}
