use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::cleanup::{dir_cleanup_reason, file_cleanup_reason};
use super::duplicates::duplicate_groups;
use super::types::{
    FileCandidate, ReclaimConfidence, ReclaimItem, ReclaimOptions, ReclaimProgress, ReclaimReport,
};
use super::util::{local_scan_threads, now_ms, systemtime_ms, to_fwd, truncate};

#[derive(Default)]
struct Acc {
    files: Vec<FileCandidate>,
    large: Vec<ReclaimItem>,
    stale: Vec<ReclaimItem>,
    empty_files: Vec<ReclaimItem>,
    empty_dirs: Vec<ReclaimItem>,
    cleanup: Vec<ReclaimItem>,
    errors: Vec<String>,
}

pub fn scan_reclaim(
    root: &Path,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
) -> ReclaimReport {
    let root_abs = root.to_path_buf();
    let acc = Arc::new(Mutex::new(Acc::default()));
    let cutoff = now_ms().saturating_sub((opts.stale_days as i64) * 86_400_000);
    let bytes = if local_scan_threads() <= 1 {
        scan_dir(&root_abs, &root_abs, progress, opts, cutoff, false, &acc)
    } else {
        match rayon::ThreadPoolBuilder::new()
            .num_threads(local_scan_threads())
            .build()
        {
            Ok(pool) => {
                pool.install(|| scan_dir(&root_abs, &root_abs, progress, opts, cutoff, false, &acc))
            }
            Err(_) => scan_dir(&root_abs, &root_abs, progress, opts, cutoff, false, &acc),
        }
    };
    let mut acc = take_acc(acc);

    acc.large.sort_by_key(|i| std::cmp::Reverse(i.size));
    acc.stale.sort_by_key(|i| std::cmp::Reverse(i.size));
    acc.empty_files.sort_by(|a, b| a.path.cmp(&b.path));
    acc.empty_dirs.sort_by(|a, b| a.path.cmp(&b.path));
    acc.cleanup.sort_by_key(|i| std::cmp::Reverse(i.size));
    truncate(&mut acc.large, opts.max_items);
    truncate(&mut acc.stale, opts.max_items);
    truncate(&mut acc.empty_files, opts.max_items);
    truncate(&mut acc.empty_dirs, opts.max_items);
    truncate(&mut acc.cleanup, opts.max_items);

    let mut duplicate_groups = duplicate_groups(acc.files, progress, opts, &mut acc.errors);
    duplicate_groups.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    truncate(&mut duplicate_groups, opts.max_items);

    ReclaimReport {
        root: to_fwd(&root_abs),
        is_remote: false,
        files: progress.files.load(Ordering::Relaxed),
        dirs: progress.dirs.load(Ordering::Relaxed),
        bytes,
        large_files: acc.large,
        stale_files: acc.stale,
        empty_files: acc.empty_files,
        empty_dirs: acc.empty_dirs,
        cleanup: acc.cleanup,
        duplicate_groups,
        errors: acc.errors,
    }
}

fn scan_dir(
    dir: &Path,
    root: &Path,
    p: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    inside_cleanup: bool,
    acc: &Arc<Mutex<Acc>>,
) -> u64 {
    if p.cancel.load(Ordering::Relaxed) || crate::agent_proto::is_pseudo_dir(&dir.to_string_lossy())
    {
        return 0;
    }
    p.dirs.fetch_add(1, Ordering::Relaxed);
    let own_cleanup = !inside_cleanup && dir_cleanup_reason(dir, root).is_some();
    let skip_detail = inside_cleanup || own_cleanup;
    let mut subdirs = Vec::new();
    let mut own_size = 0u64;
    let mut child_count = 0usize;

    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            push_error(acc, format!("{}: {}", to_fwd(dir), e));
            return 0;
        }
    };

    for ent in rd.flatten() {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        let ft = match ent.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                push_error(acc, format!("{}: {}", to_fwd(&ent.path()), e));
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        child_count += 1;
        let path = ent.path();
        if ft.is_dir() {
            subdirs.push(path);
        } else if ft.is_file() {
            let md = ent.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            p.files.fetch_add(1, Ordering::Relaxed);
            p.bytes.fetch_add(size, Ordering::Relaxed);
            own_size = own_size.saturating_add(size);
            if skip_detail {
                continue;
            }
            let mtime_ms = md
                .and_then(|m| m.modified().ok())
                .map(systemtime_ms)
                .unwrap_or(0);
            let name = ent.file_name().to_string_lossy().into_owned();
            let item = ReclaimItem::new(to_fwd(&path), name, size, mtime_ms, false);
            record_file(item, path, opts, stale_cutoff_ms, acc);
        }
    }

    let sub_size = if p.cancel.load(Ordering::Relaxed) {
        0
    } else if subdirs.len() > 1 {
        subdirs
            .par_iter()
            .map(|d| scan_dir(d, root, p, opts, stale_cutoff_ms, skip_detail, acc))
            .sum()
    } else {
        subdirs
            .iter()
            .map(|d| scan_dir(d, root, p, opts, stale_cutoff_ms, skip_detail, acc))
            .sum()
    };
    let total = own_size.saturating_add(sub_size);
    if !inside_cleanup {
        record_dir(dir, root, total, child_count, acc);
    }
    total
}

fn record_file(
    mut item: ReclaimItem,
    path: PathBuf,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    acc: &Arc<Mutex<Acc>>,
) {
    with_acc(acc, |acc| {
        if item.size >= opts.large_min_bytes {
            acc.large.push(
                item.clone()
                    .with_reason("gross", ReclaimConfidence::RiskyReview),
            );
        }
        if item.mtime_ms > 0 && item.mtime_ms < stale_cutoff_ms {
            acc.stale.push(
                item.clone()
                    .with_reason("alt", ReclaimConfidence::RiskyReview),
            );
        }
        if item.size == 0 {
            acc.empty_files.push(
                item.clone()
                    .with_reason("leer", ReclaimConfidence::ReviewSafe),
            );
        }
        if let Some(reason) = file_cleanup_reason(&item.name) {
            acc.cleanup
                .push(item.clone().with_reason(reason.reason, reason.confidence));
        }
        item.confidence = ReclaimConfidence::RiskyReview;
        acc.files.push(FileCandidate { path, item });
    });
}

fn record_dir(dir: &Path, root: &Path, size: u64, child_count: usize, acc: &Arc<Mutex<Acc>>) {
    if dir == root {
        return;
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let item = ReclaimItem::new(to_fwd(dir), name, size, 0, true);
    with_acc(acc, |acc| {
        if child_count == 0 {
            acc.empty_dirs.push(
                item.clone()
                    .with_reason("leerer Ordner", ReclaimConfidence::RiskyReview),
            );
        }
        if let Some(reason) = dir_cleanup_reason(dir, root) {
            acc.cleanup
                .push(item.with_reason(reason.reason, reason.confidence));
        }
    });
}

fn take_acc(acc: Arc<Mutex<Acc>>) -> Acc {
    match Arc::try_unwrap(acc) {
        Ok(m) => match m.into_inner() {
            Ok(acc) => acc,
            Err(poisoned) => poisoned.into_inner(),
        },
        Err(a) => match a.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                std::mem::take(&mut *guard)
            }
        },
    }
}

fn push_error(acc: &Arc<Mutex<Acc>>, error: String) {
    with_acc(acc, |acc| acc.errors.push(error));
}

fn with_acc(acc: &Arc<Mutex<Acc>>, f: impl FnOnce(&mut Acc)) {
    let mut guard = match acc.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaim_finds_duplicates_empty_and_cleanup() {
        let base = std::env::temp_dir().join(format!("se_reclaim_{}", std::process::id()));
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
        let p = ReclaimProgress::default();
        let r = scan_reclaim(&base, &p, &opts);

        assert!(r.large_files.iter().any(|i| i.name == "a.bin"));
        assert!(r.stale_files.iter().any(|i| i.name == "b.bin"));
        assert!(r.empty_files.iter().any(|i| i.name == "empty.txt"));
        assert!(r.empty_dirs.iter().any(|i| i.name == "empty_dir"));
        assert!(r
            .cleanup
            .iter()
            .any(|i| i.name == "node_modules" && i.confidence.quick_selectable()));
        assert!(r.duplicate_groups.iter().any(|g| {
            let names: std::collections::HashSet<&str> =
                g.items.iter().map(|i| i.name.as_str()).collect();
            names.contains("a.bin") && names.contains("b.bin")
        }));

        let _ = std::fs::remove_dir_all(&base);
    }
}
