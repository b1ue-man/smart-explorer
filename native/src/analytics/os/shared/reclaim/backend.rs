use crossbeam_channel::unbounded;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::Ordering;

use super::cleanup::remote_dir_cleanup_reason;
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimConfidence, ReclaimItem,
    ReclaimOptions, ReclaimProgress, ReclaimReport,
};
use super::util::{join_path, now_ms, rel_join, truncate};

#[derive(Clone)]
struct RemoteCandidate {
    item: ReclaimItem,
    md5: String,
    evidence: DuplicateEvidence,
}

#[derive(Default)]
struct BackendAcc {
    files: Vec<RemoteCandidate>,
    large: Vec<ReclaimItem>,
    stale: Vec<ReclaimItem>,
    empty_files: Vec<ReclaimItem>,
    empty_dirs: Vec<ReclaimItem>,
    cleanup: Vec<ReclaimItem>,
    errors: Vec<String>,
    bytes: u64,
}

pub fn scan_reclaim_backend(
    backend: crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
) -> ReclaimReport {
    let norm = normalize_root(root);
    let cutoff = now_ms().saturating_sub((opts.stale_days as i64) * 86_400_000);
    let mut acc = if backend.supports_walk_hashed() {
        scan_backend_via_agent(&backend, &norm, progress, opts, cutoff)
            .unwrap_or_else(|| scan_backend_listing(&backend, &norm, progress, opts, cutoff))
    } else {
        scan_backend_listing(&backend, &norm, progress, opts, cutoff)
    };

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

    let mut duplicate_groups = remote_duplicate_groups(acc.files, progress);
    duplicate_groups.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    truncate(&mut duplicate_groups, opts.max_items);

    ReclaimReport {
        root: norm,
        is_remote: true,
        files: progress.files.load(Ordering::Relaxed),
        dirs: progress.dirs.load(Ordering::Relaxed),
        bytes: acc.bytes,
        large_files: acc.large,
        stale_files: acc.stale,
        empty_files: acc.empty_files,
        empty_dirs: acc.empty_dirs,
        cleanup: acc.cleanup,
        duplicate_groups,
        errors: acc.errors,
    }
}

fn scan_backend_via_agent(
    backend: &crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
) -> Option<BackendAcc> {
    let (tx, rx) = unbounded::<crate::vfs::HashHit>();
    let mut acc = BackendAcc::default();
    let ran = std::thread::scope(|scope| {
        let h = scope.spawn(|| backend.walk_hashed(root, true, tx, &progress.cancel));
        for hit in rx.iter() {
            if progress.cancel.load(Ordering::Relaxed) {
                break;
            }
            if hit.is_dir {
                progress.dirs.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let path = join_path(root, &hit.rel);
            let name = hit
                .rel
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(hit.rel.as_str())
                .to_string();
            let item = ReclaimItem::new(path, name, hit.size, hit.mtime_ms, false);
            record_backend_file(
                item,
                hit.md5,
                DuplicateEvidence::AgentMd5,
                opts,
                stale_cutoff_ms,
                progress,
                &mut acc,
            );
        }
        h.join().unwrap_or(false)
    });
    ran.then_some(acc)
}

fn scan_backend_listing(
    backend: &crate::vfs::BackendHandle,
    root: &str,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
) -> BackendAcc {
    let mut acc = BackendAcc::default();
    let _ = scan_backend_dir(
        backend,
        root,
        String::new(),
        progress,
        opts,
        stale_cutoff_ms,
        false,
        &mut acc,
    );
    acc
}

#[allow(clippy::too_many_arguments)]
fn scan_backend_dir(
    backend: &crate::vfs::BackendHandle,
    dir: &str,
    rel_dir: String,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    inside_cleanup: bool,
    acc: &mut BackendAcc,
) -> io::Result<u64> {
    if progress.cancel.load(Ordering::Relaxed) {
        return Ok(0);
    }
    progress.dirs.fetch_add(1, Ordering::Relaxed);
    let entries = match backend.list_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            acc.errors.push(format!("{dir}: {e}"));
            return Ok(0);
        }
    };
    let own_name = dir
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let own_cleanup = !inside_cleanup && remote_dir_cleanup_reason(own_name).is_some();
    let skip_detail = inside_cleanup || own_cleanup;
    let mut own_size = 0u64;
    let mut child_count = 0usize;
    let mut subdirs = Vec::new();

    for entry in entries {
        if progress.cancel.load(Ordering::Relaxed) {
            break;
        }
        if entry.is_symlink {
            continue;
        }
        child_count += 1;
        let path = join_path(dir, &entry.name);
        let rel = rel_join(&rel_dir, &entry.name);
        if entry.is_dir {
            subdirs.push((path, rel, entry.mtime_ms, entry.name));
        } else {
            own_size = own_size.saturating_add(entry.size);
            let mut item = ReclaimItem::new(path, entry.name, entry.size, entry.mtime_ms, false);
            item.backend_id = entry.id;
            record_backend_file(
                item,
                entry.content_md5,
                DuplicateEvidence::ProviderMd5,
                opts,
                stale_cutoff_ms,
                progress,
                acc,
            );
        }
    }

    let mut sub_size = 0u64;
    for (path, rel, mtime_ms, name) in subdirs {
        let size = scan_backend_dir(
            backend,
            &path,
            rel,
            progress,
            opts,
            stale_cutoff_ms,
            skip_detail,
            acc,
        )?;
        sub_size = sub_size.saturating_add(size);
        if !inside_cleanup {
            record_backend_dir(path, name, size, mtime_ms, child_count, acc);
        }
    }
    Ok(own_size.saturating_add(sub_size))
}

fn record_backend_file(
    mut item: ReclaimItem,
    md5: Option<String>,
    evidence: DuplicateEvidence,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    progress: &ReclaimProgress,
    acc: &mut BackendAcc,
) {
    progress.files.fetch_add(1, Ordering::Relaxed);
    progress.bytes.fetch_add(item.size, Ordering::Relaxed);
    acc.bytes = acc.bytes.saturating_add(item.size);
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
    if item.size >= opts.duplicate_min_bytes {
        if let Some(md5) = md5.filter(|h| h.len() >= 32 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            item.reason = "Duplikat".to_string();
            item.confidence = ReclaimConfidence::HashMatch;
            acc.files.push(RemoteCandidate {
                item,
                md5: md5.to_ascii_lowercase(),
                evidence,
            });
        }
    }
}

fn record_backend_dir(
    path: String,
    name: String,
    size: u64,
    mtime_ms: i64,
    child_count: usize,
    acc: &mut BackendAcc,
) {
    let item = ReclaimItem::new(path, name.clone(), size, mtime_ms, true);
    if child_count == 0 {
        acc.empty_dirs.push(
            item.clone()
                .with_reason("leerer Ordner", ReclaimConfidence::RiskyReview),
        );
    }
    if let Some(reason) = remote_dir_cleanup_reason(&name) {
        acc.cleanup
            .push(item.with_reason(reason.reason, reason.confidence));
    }
}

fn remote_duplicate_groups(
    files: Vec<RemoteCandidate>,
    progress: &ReclaimProgress,
) -> Vec<DuplicateGroup> {
    let mut by_key: HashMap<(u64, String, DuplicateEvidence), Vec<ReclaimItem>> = HashMap::new();
    for f in files {
        by_key
            .entry((f.item.size, f.md5, f.evidence))
            .or_default()
            .push(f.item);
    }
    let mut groups = Vec::new();
    for ((size, md5, evidence), mut items) in by_key.into_iter().filter(|(_, v)| v.len() > 1) {
        items.sort_by_key(|i| std::cmp::Reverse(i.mtime_ms));
        let reclaimable = size.saturating_mul(items.len().saturating_sub(1) as u64);
        progress
            .candidates
            .fetch_add(items.len() as u64, Ordering::Relaxed);
        groups.push(DuplicateGroup {
            hash: ContentHash {
                algorithm: HashAlgorithm::Md5,
                hex: md5,
            },
            evidence,
            size,
            reclaimable,
            items,
        });
    }
    groups
}

fn normalize_root(root: &str) -> String {
    let t = root.trim_end_matches('/');
    if t.is_empty() {
        "/".to_string()
    } else {
        t.to_string()
    }
}
