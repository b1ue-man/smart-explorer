use crossbeam_channel::bounded;
use std::sync::atomic::{AtomicBool, Ordering};

use super::backend_duplicates::remote_duplicate_groups;
use super::budget::{LimitExceeded, ReclaimBudget};
use super::cleanup::remote_dir_cleanup_reason;
use super::retention::{compare_item_path, compare_item_size, retain_best};
use super::types::{
    DuplicateEvidence, ReclaimConfidence, ReclaimItem, ReclaimOptions, ReclaimProgress,
    ReclaimReport, ReclaimResultCounts,
};
use super::util::{join_path, now_ms, push_bounded_error, rel_join, stale_cutoff_ms};

#[derive(Clone)]
pub(super) struct RemoteCandidate {
    pub(super) item: ReclaimItem,
    pub(super) md5: String,
    pub(super) evidence: DuplicateEvidence,
}

#[derive(Default)]
struct BackendAcc {
    files: Vec<RemoteCandidate>,
    large: Vec<ReclaimItem>,
    stale: Vec<ReclaimItem>,
    empty_files: Vec<ReclaimItem>,
    empty_dirs: Vec<ReclaimItem>,
    cleanup: Vec<ReclaimItem>,
    result_counts: ReclaimResultCounts,
    duplicate_candidates: u64,
    errors: Vec<String>,
    root_error: Option<String>,
    scan_limit: Option<String>,
    suppressed_errors: u64,
    bytes: u64,
}

#[derive(Default)]
struct DirScan {
    bytes: u64,
    children: usize,
    complete: bool,
}

pub fn scan_reclaim_backend(
    backend: crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
) -> ReclaimReport {
    let norm = normalize_root(root);
    let cutoff = stale_cutoff_ms(now_ms(), opts.stale_days);
    let mut budget = ReclaimBudget::default();
    let mut acc = if backend.supports_walk_hashed() {
        match scan_backend_via_agent(&backend, &norm, progress, opts, cutoff, &mut budget) {
            Some(acc) => acc,
            None => scan_backend_listing(&backend, &norm, progress, opts, cutoff, &mut budget),
        }
    } else {
        scan_backend_listing(&backend, &norm, progress, opts, cutoff, &mut budget)
    };

    acc.large.sort_by(compare_item_size);
    acc.stale.sort_by(compare_item_size);
    acc.empty_files.sort_by(compare_item_path);
    acc.empty_dirs.sort_by(compare_item_path);
    acc.cleanup.sort_by(compare_item_size);
    let duplicate_candidates_retained = acc.files.len() as u64;
    let analysis = remote_duplicate_groups(acc.files, progress, opts.max_items);
    acc.result_counts.duplicate_groups = analysis.total_groups;

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
        duplicate_groups: analysis.groups,
        duplicate_candidates: acc.duplicate_candidates,
        duplicate_candidates_retained,
        errors: acc.errors,
        suppressed_errors: acc.suppressed_errors,
    }
}

fn scan_backend_via_agent(
    backend: &crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    budget: &mut ReclaimBudget,
) -> Option<BackendAcc> {
    let (tx, rx) = bounded::<crate::vfs::HashHit>(1024);
    let walk_cancel = AtomicBool::new(false);
    let worker_cancel = &walk_cancel;
    let mut acc = BackendAcc::default();
    let mut received = false;
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(move || backend.walk_hashed(root, true, tx, worker_cancel));
        for hit in rx.iter() {
            received = true;
            if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
                walk_cancel.store(true, Ordering::Relaxed);
                continue;
            }
            let path = join_path(root, &hit.rel);
            let name = hit
                .rel
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(hit.rel.as_str())
                .to_string();
            let depth = u32::try_from(
                hit.rel
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .count(),
            )
            .unwrap_or(u32::MAX);
            if let Err(limit) = budget.claim(path.len().saturating_add(name.len()), depth) {
                record_limit(&mut acc, root, limit);
                walk_cancel.store(true, Ordering::Relaxed);
                continue;
            }
            if hit.is_dir {
                progress.dirs.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let item = ReclaimItem::new(path, name, hit.size, hit.mtime_ms, false);
            record_backend_file(
                item,
                hit.md5,
                DuplicateEvidence::AgentMd5,
                opts,
                stale_cutoff_ms,
                progress,
                true,
                &mut acc,
            );
        }
        worker.join()
    });
    match outcome {
        Ok(Ok(true)) => Some(acc),
        Ok(Ok(false))
            if !received && !budget.stopped() && !progress.cancel.load(Ordering::Relaxed) =>
        {
            None
        }
        Ok(Ok(false)) if budget.stopped() || progress.cancel.load(Ordering::Relaxed) => Some(acc),
        Ok(Ok(false)) => {
            record_agent_walk_error(
                &mut acc,
                root,
                "backend reported unsupported after streaming entries",
            );
            Some(acc)
        }
        Ok(Err(_)) if budget.stopped() || progress.cancel.load(Ordering::Relaxed) => Some(acc),
        Ok(Err(error)) => {
            record_agent_walk_error(&mut acc, root, &error.to_string());
            Some(acc)
        }
        Err(_) => {
            record_agent_walk_error(&mut acc, root, "server-side walk worker panicked");
            Some(acc)
        }
    }
}

fn record_agent_walk_error(acc: &mut BackendAcc, root: &str, message: &str) {
    let error = format!("{root}: agent hash walk failed: {message}");
    acc.root_error = Some(error.clone());
    push_bounded_error(&mut acc.errors, &mut acc.suppressed_errors, error);
}

fn scan_backend_listing(
    backend: &crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    budget: &mut ReclaimBudget,
) -> BackendAcc {
    let mut acc = BackendAcc::default();
    let _ = scan_backend_dir(
        backend,
        root,
        "",
        progress,
        opts,
        stale_cutoff_ms,
        false,
        0,
        budget,
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
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    inside_cleanup: bool,
    depth: u32,
    budget: &mut ReclaimBudget,
    acc: &mut BackendAcc,
) -> DirScan {
    if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
        return DirScan::default();
    }
    progress.dirs.fetch_add(1, Ordering::Relaxed);
    let mut entries = match backend.list_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            let error = format!("{dir}: {error}");
            if rel_dir.is_empty() && acc.root_error.is_none() {
                acc.root_error = Some(error.clone());
            }
            push_bounded_error(&mut acc.errors, &mut acc.suppressed_errors, error);
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
        if entry.is_symlink {
            continue;
        }
        if entry.is_dir {
            let rel = rel_join(rel_dir, &entry.name);
            let child = scan_backend_dir(
                backend,
                &path,
                &rel,
                progress,
                opts,
                stale_cutoff_ms,
                skip_detail,
                depth.saturating_add(1),
                budget,
                acc,
            );
            result.bytes = result.bytes.saturating_add(child.bytes);
            result.complete &= child.complete;
            if !skip_detail && child.complete {
                record_backend_dir(
                    path,
                    entry.name,
                    child.bytes,
                    entry.mtime_ms,
                    child.children,
                    opts.max_items,
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
                opts,
                stale_cutoff_ms,
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

#[allow(clippy::too_many_arguments)]
fn record_backend_file(
    mut item: ReclaimItem,
    md5: Option<String>,
    evidence: DuplicateEvidence,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
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
    if item.size >= opts.large_min_bytes {
        acc.result_counts.large_files = acc.result_counts.large_files.saturating_add(1);
        retain_best(
            &mut acc.large,
            item.clone()
                .with_reason("gross", ReclaimConfidence::RiskyReview),
            opts.max_items,
            compare_item_size,
        );
    }
    if item.mtime_ms > 0 && item.mtime_ms < stale_cutoff_ms {
        acc.result_counts.stale_files = acc.result_counts.stale_files.saturating_add(1);
        retain_best(
            &mut acc.stale,
            item.clone()
                .with_reason("alt", ReclaimConfidence::RiskyReview),
            opts.max_items,
            compare_item_size,
        );
    }
    if item.size == 0 {
        acc.result_counts.empty_files = acc.result_counts.empty_files.saturating_add(1);
        retain_best(
            &mut acc.empty_files,
            item.clone()
                .with_reason("leer", ReclaimConfidence::ReviewSafe),
            opts.max_items,
            compare_item_path,
        );
    }
    if item.size >= opts.duplicate_min_bytes {
        if let Some(md5) =
            md5.filter(|hash| hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            acc.duplicate_candidates = acc.duplicate_candidates.saturating_add(1);
            item.reason = "Duplikat".to_string();
            item.confidence = ReclaimConfidence::HashMatch;
            retain_best(
                &mut acc.files,
                RemoteCandidate {
                    item,
                    md5: md5.to_ascii_lowercase(),
                    evidence,
                },
                opts.max_items,
                |left, right| {
                    right
                        .item
                        .size
                        .cmp(&left.item.size)
                        .then_with(|| left.item.path.cmp(&right.item.path))
                },
            );
        }
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

fn record_limit(acc: &mut BackendAcc, root: &str, limit: LimitExceeded) {
    if acc.scan_limit.is_some() {
        return;
    }
    let detail = limit.to_string();
    acc.scan_limit = Some(detail.clone());
    push_bounded_error(
        &mut acc.errors,
        &mut acc.suppressed_errors,
        format!("{root}: reclaim scan stopped at {detail}"),
    );
}

fn normalize_root(root: &str) -> String {
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}
