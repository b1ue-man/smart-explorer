//! Find-and-reclaim and duplicate search of a remote location. An agent
//! that hashes next to the data (SSH) streams sizes and MD5 in one walk;
//! every other backend is listed folder by folder, its provider hashes are
//! used where it has them and the rest is compared by content, reading only
//! files that share their size with another one.
use std::path::Path;
use std::sync::atomic::Ordering;

use super::backend_agent::{hashes_server_side, scan_backend_via_agent};
use super::backend_duplicates::{duplicate_groups, Candidate, Candidates};
use super::budget::{describe_scan_limit, LimitExceeded, ReclaimBudget};
use super::cleanup::remote_dir_cleanup_reason;
use super::finder::{DuplicateReport, DuplicateSummary, MAX_CANDIDATE_TEXT_BYTES};
use super::retention::{compare_item_path, compare_item_size, retain_best};
use super::types::{
    DuplicateEvidence, ReclaimConfidence, ReclaimItem, ReclaimOptions, ReclaimProgress,
    ReclaimReport, ReclaimResultCounts,
};
use super::util::{join_path, now_ms, push_bounded_error, rel_join, stale_cutoff_ms};
use crate::analytics::thousands;

/// What one walk collects.
pub(super) struct Walk<'a> {
    opts: &'a ReclaimOptions,
    stale_cutoff_ms: i64,
    /// Large, stale and empty entries and cleanup folders (find-and-reclaim);
    /// a duplicate search only collects candidates.
    reclaim_lists: bool,
}

pub(super) struct BackendAcc {
    candidates: Candidates,
    large: Vec<ReclaimItem>,
    stale: Vec<ReclaimItem>,
    empty_files: Vec<ReclaimItem>,
    empty_dirs: Vec<ReclaimItem>,
    cleanup: Vec<ReclaimItem>,
    result_counts: ReclaimResultCounts,
    errors: Vec<String>,
    root_error: Option<String>,
    scan_limit: Option<String>,
    suppressed_errors: u64,
    bytes: u64,
}

impl BackendAcc {
    pub(super) fn new() -> Self {
        Self {
            candidates: Candidates::new(MAX_CANDIDATE_TEXT_BYTES),
            large: Vec::new(),
            stale: Vec::new(),
            empty_files: Vec::new(),
            empty_dirs: Vec::new(),
            cleanup: Vec::new(),
            result_counts: ReclaimResultCounts::default(),
            errors: Vec::new(),
            root_error: None,
            scan_limit: None,
            suppressed_errors: 0,
            bytes: 0,
        }
    }

    fn error(&mut self, error: String) {
        push_bounded_error(&mut self.errors, &mut self.suppressed_errors, error);
    }
}

#[derive(Default)]
struct DirScan {
    bytes: u64,
    children: usize,
    complete: bool,
}

/// Find-and-reclaim of the desktop: candidates, large/stale/empty entries,
/// cleanup folders and the `opts.max_items` largest duplicate groups.
pub fn scan_reclaim_backend(
    backend: crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
) -> ReclaimReport {
    let norm = normalize_root(root);
    let walk = Walk {
        opts,
        stale_cutoff_ms: stale_cutoff_ms(now_ms(), opts.stale_days),
        reclaim_lists: true,
    };
    let mut acc = walk_backend(&backend, &norm, progress, &walk);
    acc.large.sort_by(compare_item_size);
    acc.stale.sort_by(compare_item_size);
    acc.empty_files.sort_by(compare_item_path);
    acc.empty_dirs.sort_by(compare_item_path);
    acc.cleanup.sort_by(compare_item_size);
    let duplicate_candidates = acc.candidates.seen();
    let duplicate_candidates_retained = acc.candidates.len();
    let found = duplicate_groups(&*backend, acc.candidates.take(), progress, opts.max_items);
    for error in found.errors {
        acc.error(error);
    }
    acc.result_counts.duplicate_groups = found.total_groups;

    ReclaimReport {
        root: norm,
        is_remote: true,
        root_error: acc.root_error,
        scan_limit: acc.scan_limit,
        files: progress.files.load(Ordering::Relaxed),
        dirs: progress.dirs.load(Ordering::Relaxed),
        bytes: acc.bytes,
        large_min_bytes: opts.large_min_bytes,
        stale_days: opts.stale_days,
        result_counts: acc.result_counts,
        large_files: acc.large,
        stale_files: acc.stale,
        empty_files: acc.empty_files,
        empty_dirs: acc.empty_dirs,
        cleanup: acc.cleanup,
        duplicate_groups: found.groups,
        duplicate_candidates,
        duplicate_candidates_retained,
        errors: acc.errors,
        suppressed_errors: acc.suppressed_errors,
    }
}

/// Duplicate search of a remote location (the Android app's `reclaim.start`):
/// every file of at least `min_bytes` is a candidate and every group is
/// returned, in the shape of the local search.
pub fn find_backend_duplicates(
    backend: crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    min_bytes: u64,
) -> DuplicateReport {
    let opts = ReclaimOptions {
        duplicate_min_bytes: min_bytes.max(1),
        ..ReclaimOptions::default()
    };
    let walk = Walk {
        opts: &opts,
        stale_cutoff_ms: i64::MIN,
        reclaim_lists: false,
    };
    let mut acc = walk_backend(&backend, &normalize_root(root), progress, &walk);
    let candidates = acc.candidates.seen();
    let dropped = acc.candidates.dropped();
    let found = duplicate_groups(&*backend, acc.candidates.take(), progress, usize::MAX);
    for error in found.errors {
        acc.error(error);
    }
    let mut limits = Vec::new();
    if let Some(raw) = &acc.scan_limit {
        limits.push(describe_scan_limit(raw));
    }
    if dropped > 0 {
        limits.push(format!(
            "Kandidatenspeicher ({} MiB) ausgeschöpft: die {} kleinsten Dateien nicht verglichen",
            MAX_CANDIDATE_TEXT_BYTES / (1024 * 1024),
            thousands(dropped)
        ));
    }
    DuplicateReport {
        summary: DuplicateSummary {
            files: progress.files.load(Ordering::Relaxed),
            bytes: acc.bytes,
            candidates,
            compared: found.compared,
            groups: found.groups.len() as u64,
            protected: Vec::new(),
            errors: acc.errors,
            suppressed_errors: acc.suppressed_errors,
            limits,
        },
        groups: found.groups,
        root_error: acc.root_error,
    }
}

fn walk_backend(
    backend: &crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    walk: &Walk<'_>,
) -> BackendAcc {
    if hashes_server_side(&**backend) {
        let mut budget = ReclaimBudget::default();
        if let Some(acc) = scan_backend_via_agent(backend, root, progress, walk, &mut budget) {
            return acc;
        }
    }
    let mut budget = ReclaimBudget::default();
    let mut acc = BackendAcc::new();
    let _ = scan_backend_dir(
        backend,
        root,
        "",
        progress,
        walk,
        false,
        0,
        &mut budget,
        &mut acc,
    );
    acc
}

#[allow(clippy::too_many_arguments)]
fn scan_backend_dir(
    backend: &crate::vfs::BackendHandle,
    dir: &str,
    rel_dir: &str,
    progress: &ReclaimProgress,
    walk: &Walk<'_>,
    inside_cleanup: bool,
    depth: u32,
    budget: &mut ReclaimBudget,
    acc: &mut BackendAcc,
) -> DirScan {
    if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
        return DirScan::default();
    }
    progress.dirs.fetch_add(1, Ordering::Relaxed);
    progress.stage.enter_directory(Path::new(dir));
    let mut entries = match backend.list_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            let error = format!("{dir}: {error}");
            if rel_dir.is_empty() && acc.root_error.is_none() {
                acc.root_error = Some(error.clone());
            }
            acc.error(error);
            return DirScan::default();
        }
    };
    entries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| right.is_dir.cmp(&left.is_dir))
    });
    let own_name = dir
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let own_cleanup = !inside_cleanup && remote_dir_cleanup_reason(own_name).is_some();
    let skip_detail = inside_cleanup || own_cleanup;
    let mut result = DirScan {
        bytes: 0,
        children: 0,
        complete: true,
    };

    for entry in entries {
        if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
            result.complete = false;
            break;
        }
        let path = join_path(dir, &entry.name);
        if let Err(limit) = budget.claim(
            path.len().saturating_add(entry.name.len()),
            depth.saturating_add(1),
        ) {
            result.complete = false;
            record_limit(acc, dir, limit);
            break;
        }
        result.children = result.children.saturating_add(1);
        if entry.is_symlink || entry.special || crate::apptrash::excluded_name(&entry.name) {
            continue;
        }
        if entry.is_dir {
            let rel = rel_join(rel_dir, &entry.name);
            let child = scan_backend_dir(
                backend,
                &path,
                &rel,
                progress,
                walk,
                skip_detail,
                depth.saturating_add(1),
                budget,
                acc,
            );
            result.bytes = result.bytes.saturating_add(child.bytes);
            result.complete &= child.complete;
            if walk.reclaim_lists && !skip_detail && child.complete {
                record_backend_dir(
                    path,
                    entry.name,
                    child.bytes,
                    entry.mtime_ms,
                    child.children,
                    walk.opts.max_items,
                    acc,
                );
            }
        } else {
            result.bytes = result.bytes.saturating_add(entry.size);
            let mut item = ReclaimItem::new(path, entry.name, entry.size, entry.mtime_ms, false);
            item.backend_id = entry.id;
            record_backend_file(
                item,
                entry.content_md5,
                DuplicateEvidence::ProviderMd5,
                walk,
                progress,
                !skip_detail,
                acc,
            );
        }
    }
    if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
        result.complete = false;
    }
    result
}

/// Counts one file; with `collect_detail` (not below a cleanup folder) it
/// may become a reclaim item and, at the minimum size, a duplicate candidate
/// with the backend's hash when there is a valid one.
pub(super) fn record_backend_file(
    item: ReclaimItem,
    md5: Option<String>,
    evidence: DuplicateEvidence,
    walk: &Walk<'_>,
    progress: &ReclaimProgress,
    collect_detail: bool,
    acc: &mut BackendAcc,
) {
    progress.files.fetch_add(1, Ordering::Relaxed);
    let _ = progress
        .bytes
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(item.size))
        });
    acc.bytes = acc.bytes.saturating_add(item.size);
    if !collect_detail {
        return;
    }
    if walk.reclaim_lists {
        record_reclaim_lists(&item, walk, acc);
    }
    if item.size >= walk.opts.duplicate_min_bytes {
        let hash = md5
            .filter(|hash| hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(|hash| (hash.to_ascii_lowercase(), evidence));
        acc.candidates.offer(Candidate {
            item: item.with_reason("Duplikat", ReclaimConfidence::HashMatch),
            hash,
        });
    }
}

fn record_reclaim_lists(item: &ReclaimItem, walk: &Walk<'_>, acc: &mut BackendAcc) {
    let limit = walk.opts.max_items;
    if item.size >= walk.opts.large_min_bytes {
        acc.result_counts.large_files = acc.result_counts.large_files.saturating_add(1);
        retain_best(
            &mut acc.large,
            item.clone()
                .with_reason("gross", ReclaimConfidence::RiskyReview),
            limit,
            compare_item_size,
        );
    }
    if item.mtime_ms > 0 && item.mtime_ms < walk.stale_cutoff_ms {
        acc.result_counts.stale_files = acc.result_counts.stale_files.saturating_add(1);
        retain_best(
            &mut acc.stale,
            item.clone()
                .with_reason("alt", ReclaimConfidence::RiskyReview),
            limit,
            compare_item_size,
        );
    }
    if item.size == 0 {
        acc.result_counts.empty_files = acc.result_counts.empty_files.saturating_add(1);
        retain_best(
            &mut acc.empty_files,
            item.clone()
                .with_reason("leer", ReclaimConfidence::ReviewSafe),
            limit,
            compare_item_path,
        );
    }
}

fn record_backend_dir(
    path: String,
    name: String,
    size: u64,
    mtime_ms: i64,
    child_count: usize,
    limit: usize,
    acc: &mut BackendAcc,
) {
    let item = ReclaimItem::new(path, name.clone(), size, mtime_ms, true);
    if child_count == 0 {
        acc.result_counts.empty_dirs = acc.result_counts.empty_dirs.saturating_add(1);
        retain_best(
            &mut acc.empty_dirs,
            item.clone()
                .with_reason("leerer Ordner", ReclaimConfidence::RiskyReview),
            limit,
            compare_item_path,
        );
    }
    if let Some(reason) = remote_dir_cleanup_reason(&name) {
        acc.result_counts.cleanup = acc.result_counts.cleanup.saturating_add(1);
        retain_best(
            &mut acc.cleanup,
            item.with_reason(reason.reason, reason.confidence),
            limit,
            compare_item_size,
        );
    }
}

pub(super) fn record_limit(acc: &mut BackendAcc, root: &str, limit: LimitExceeded) {
    if acc.scan_limit.is_some() {
        return;
    }
    let detail = limit.to_string();
    acc.scan_limit = Some(detail.clone());
    acc.error(format!("{root}: reclaim scan stopped at {detail}"));
}

fn normalize_root(root: &str) -> String {
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}
