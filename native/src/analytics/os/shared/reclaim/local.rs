use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::budget::{LimitExceeded, ReclaimBudget};
use super::cleanup::{dir_cleanup_reason, file_cleanup_reason};
use super::duplicates::duplicate_groups;
use super::retention::{compare_item_path, compare_item_size, retain_best};
use super::types::{
    FileCandidate, ReclaimConfidence, ReclaimItem, ReclaimOptions, ReclaimProgress, ReclaimReport,
    ReclaimResultCounts,
};
use super::util::{now_ms, push_bounded_error, stale_cutoff_ms, systemtime_ms, to_fwd};

#[derive(Default)]
struct Acc {
    files: Vec<FileCandidate>,
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
}

#[derive(Default)]
struct DirScan {
    bytes: u64,
    complete: bool,
}

pub fn scan_reclaim(
    root: &Path,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
) -> ReclaimReport {
    let root_abs = root.to_path_buf();
    let mut acc = Acc::default();
    let mut budget = ReclaimBudget::default();
    let cutoff = stale_cutoff_ms(now_ms(), opts.stale_days);
    let scanned = scan_dir(
        &root_abs,
        &root_abs,
        progress,
        opts,
        cutoff,
        false,
        0,
        &mut budget,
        &mut acc,
    );

    acc.large.sort_by(compare_item_size);
    acc.stale.sort_by(compare_item_size);
    acc.empty_files.sort_by(compare_item_path);
    acc.empty_dirs.sort_by(compare_item_path);
    acc.cleanup.sort_by(compare_item_size);
    let duplicate_candidates_retained = acc.files.len() as u64;
    let analysis = duplicate_groups(
        acc.files,
        progress,
        opts,
        &mut acc.errors,
        &mut acc.suppressed_errors,
    );
    acc.result_counts.duplicate_groups = analysis.total_groups;

    ReclaimReport {
        root: to_fwd(&root_abs),
        is_remote: false,
        root_error: acc.root_error,
        scan_limit: acc.scan_limit,
        files: progress.files.load(Ordering::Relaxed),
        dirs: progress.dirs.load(Ordering::Relaxed),
        bytes: scanned.bytes,
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

#[allow(clippy::too_many_arguments)]
fn scan_dir(
    dir: &Path,
    root: &Path,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    inside_cleanup: bool,
    depth: u32,
    budget: &mut ReclaimBudget,
    acc: &mut Acc,
) -> DirScan {
    if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
        return DirScan::default();
    }
    if crate::agent_proto::is_pseudo_dir(&dir.to_string_lossy()) {
        return DirScan {
            bytes: 0,
            complete: true,
        };
    }
    progress.dirs.fetch_add(1, Ordering::Relaxed);
    let own_cleanup = !inside_cleanup && dir_cleanup_reason(dir, root).is_some();
    let skip_detail = inside_cleanup || own_cleanup;
    let mut result = DirScan {
        bytes: 0,
        complete: true,
    };
    let mut child_count = 0usize;
    let read_dir = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            push_error(acc, format!("{}: {error}", to_fwd(dir)), dir == root);
            return DirScan::default();
        }
    };

    for entry in read_dir {
        if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
            result.complete = false;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                result.complete = false;
                push_error(
                    acc,
                    format!("{}: directory entry: {error}", to_fwd(dir)),
                    false,
                );
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name();
        let text_bytes = path.to_string_lossy().len().saturating_add(name.len());
        if let Err(limit) = budget.claim(text_bytes, depth.saturating_add(1)) {
            result.complete = false;
            record_limit(acc, root, limit);
            break;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                result.complete = false;
                push_error(acc, format!("{}: {error}", to_fwd(&path)), false);
                continue;
            }
        };
        child_count = child_count.saturating_add(1);
        if file_type.is_symlink()
            || (file_type.is_dir() && crate::agent_proto::is_pseudo_dir(&path.to_string_lossy()))
        {
            continue;
        }
        if file_type.is_dir() {
            let child = scan_dir(
                &path,
                root,
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
        } else if file_type.is_file() {
            progress.files.fetch_add(1, Ordering::Relaxed);
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    result.complete = false;
                    push_error(acc, format!("{}: {error}", to_fwd(&path)), false);
                    continue;
                }
            };
            let size = metadata.len();
            let _ = progress
                .bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    Some(current.saturating_add(size))
                });
            result.bytes = result.bytes.saturating_add(size);
            if skip_detail {
                continue;
            }
            let mtime_ms = metadata.modified().ok().map(systemtime_ms).unwrap_or(0);
            let item = ReclaimItem::new(
                to_fwd(&path),
                name.to_string_lossy().into_owned(),
                size,
                mtime_ms,
                false,
            );
            record_file(item, path, opts, stale_cutoff_ms, acc);
        }
    }

    if progress.cancel.load(Ordering::Relaxed) || budget.stopped() {
        result.complete = false;
    }
    if !inside_cleanup && result.complete {
        record_dir(dir, root, result.bytes, child_count, opts.max_items, acc);
    }
    result
}

fn record_file(
    mut item: ReclaimItem,
    path: PathBuf,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    acc: &mut Acc,
) {
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
    if let Some(reason) = file_cleanup_reason(&item.name) {
        acc.result_counts.cleanup = acc.result_counts.cleanup.saturating_add(1);
        retain_best(
            &mut acc.cleanup,
            item.clone().with_reason(reason.reason, reason.confidence),
            opts.max_items,
            compare_item_size,
        );
    }
    if item.size >= opts.duplicate_min_bytes {
        acc.duplicate_candidates = acc.duplicate_candidates.saturating_add(1);
        item.confidence = ReclaimConfidence::RiskyReview;
        retain_best(
            &mut acc.files,
            FileCandidate { path, item },
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

fn record_dir(dir: &Path, root: &Path, size: u64, child_count: usize, limit: usize, acc: &mut Acc) {
    if dir == root {
        return;
    }
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let item = ReclaimItem::new(to_fwd(dir), name, size, 0, true);
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
    if let Some(reason) = dir_cleanup_reason(dir, root) {
        acc.result_counts.cleanup = acc.result_counts.cleanup.saturating_add(1);
        retain_best(
            &mut acc.cleanup,
            item.with_reason(reason.reason, reason.confidence),
            limit,
            compare_item_size,
        );
    }
}

fn record_limit(acc: &mut Acc, root: &Path, limit: LimitExceeded) {
    if acc.scan_limit.is_some() {
        return;
    }
    let detail = limit.to_string();
    acc.scan_limit = Some(detail.clone());
    push_bounded_error(
        &mut acc.errors,
        &mut acc.suppressed_errors,
        format!("{}: reclaim scan stopped at {detail}", to_fwd(root)),
    );
}

fn push_error(acc: &mut Acc, error: String, is_root: bool) {
    if is_root && acc.root_error.is_none() {
        acc.root_error = Some(error.clone());
    }
    push_bounded_error(&mut acc.errors, &mut acc.suppressed_errors, error);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{name}_{}", std::process::id()))
    }

    #[test]
    fn reclaim_finds_duplicates_empty_and_cleanup() {
        let base = temp_base("se_reclaim");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(base.join("empty_dir")).unwrap();
        std::fs::write(base.join("package.json"), "{}").unwrap();
        std::fs::write(base.join("package-lock.json"), "{}").unwrap();
        std::fs::write(base.join("a.bin"), b"same").unwrap();
        std::fs::write(base.join("b.bin"), b"same").unwrap();
        std::fs::write(base.join("empty.txt"), b"").unwrap();
        std::fs::write(base.join("node_modules/pkg/cache.js"), b"cached").unwrap();

        let opts = ReclaimOptions {
            large_min_bytes: 1,
            stale_days: 0,
            max_items: 50,
            duplicate_min_bytes: 1,
            partial_fingerprint_bytes: 2,
        };
        let report = scan_reclaim(&base, &ReclaimProgress::default(), &opts);

        assert!(report.large_files.iter().any(|item| item.name == "a.bin"));
        assert!(report.stale_files.iter().any(|item| item.name == "b.bin"));
        assert!(report
            .empty_files
            .iter()
            .any(|item| item.name == "empty.txt"));
        assert!(report
            .empty_dirs
            .iter()
            .any(|item| item.name == "empty_dir"));
        assert!(report
            .cleanup
            .iter()
            .any(|item| { item.name == "node_modules" && item.confidence.quick_selectable() }));
        assert!(report.duplicate_groups.iter().any(|group| {
            let names: std::collections::HashSet<&str> =
                group.items.iter().map(|item| item.name.as_str()).collect();
            names.contains("a.bin") && names.contains("b.bin")
        }));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn max_items_bounds_categories_and_duplicate_inputs_during_walk() {
        let base = temp_base("se_reclaim_bounded");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("largest-a.bin"), vec![7; 32]).unwrap();
        std::fs::write(base.join("largest-b.bin"), vec![7; 32]).unwrap();
        for index in 1..=6 {
            std::fs::write(
                base.join(format!("small-{index}.bin")),
                vec![index as u8; index],
            )
            .unwrap();
        }
        let opts = ReclaimOptions {
            large_min_bytes: 1,
            stale_days: 0,
            max_items: 2,
            duplicate_min_bytes: 1,
            partial_fingerprint_bytes: 4,
        };
        let report = scan_reclaim(&base, &ReclaimProgress::default(), &opts);

        assert_eq!(report.result_counts.large_files, 8);
        assert_eq!(report.large_files.len(), 2);
        assert_eq!(report.duplicate_candidates, 8);
        assert_eq!(report.duplicate_candidates_retained, 2);
        assert!(report.duplicate_candidates_truncated());
        assert_eq!(report.result_counts.duplicate_groups, 1);
        assert_eq!(report.duplicate_groups.len(), 1);
        assert!(report
            .large_files
            .iter()
            .all(|item| item.name.starts_with("largest-")));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_reclaim_root_is_explicit() {
        let base = temp_base("se_reclaim_missing");
        let _ = std::fs::remove_dir_all(&base);
        let report = scan_reclaim(
            &base,
            &ReclaimProgress::default(),
            &ReclaimOptions::default(),
        );
        assert!(report.root_error.is_some());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.suppressed_errors, 0);
    }
}
