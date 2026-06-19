use crate::types::{Conflict, CopyMode, CopyOptions, CopyProgress, FileEntry};
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub enum CopyMsg {
    Progress(CopyProgress),
    Done {
        progress: CopyProgress,
        errors: Vec<(String, String)>,
    },
}

pub struct CopyHandle {
    pub cancel: Arc<AtomicBool>,
}

fn unique_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = target
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut i = 2;
    loop {
        let candidate = parent.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn rel_from_root(path_fwd: &str, root_fwd: &str) -> String {
    if path_fwd.starts_with(root_fwd) {
        let rel = path_fwd[root_fwd.len()..].trim_start_matches('/');
        if rel.is_empty() {
            // path equals root → use basename
            return path_fwd.rsplit('/').next().unwrap_or(path_fwd).to_string();
        }
        rel.to_string()
    } else {
        path_fwd.rsplit('/').next().unwrap_or(path_fwd).to_string()
    }
}

pub fn start_copy(
    entries: Vec<FileEntry>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            run_copy(entries, opts, tx, cancel_clone);
        })
        .expect("spawn copy thread");

    CopyHandle { cancel }
}

/// Like `start_copy`, but the directory expansion (recursive walk of selected
/// folders) happens on the worker thread, so the UI never blocks on a large
/// subtree. `filter` (with its root prefix) is applied to the expanded
/// entries; selected plain files always pass.
pub fn start_copy_expanded(
    seeds: Vec<FileEntry>,
    filter: Option<(crate::types::FilterDef, String)>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let cf = filter
                .as_ref()
                .map(|(f, prefix)| (crate::filter::CompiledFilter::compile(f), prefix.clone()));
            let mut entries: Vec<FileEntry> = Vec::new();
            for e in &seeds {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                if e.is_dir {
                    let sub = crate::scanner::collect_recursive(
                        &PathBuf::from(e.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                        false,
                        e.depth + 1,
                    );
                    entries.push(e.clone());
                    match &cf {
                        Some((cf, prefix)) => {
                            entries.extend(sub.into_iter().filter(|s| cf.matches(s, prefix)))
                        }
                        None => entries.extend(sub),
                    }
                } else {
                    entries.push(e.clone());
                }
            }
            run_copy(entries, opts, tx, cancel_clone);
        })
        .expect("spawn copy thread");

    CopyHandle { cancel }
}

/// Copy/move raw clipboard paths into a destination. Stats and expands the
/// paths on the worker thread (the previous implementation did this on the
/// UI thread and froze on big folders).
pub fn start_copy_from_paths(
    paths: Vec<String>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let mut entries: Vec<FileEntry> = Vec::new();
            for p in &paths {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                let pb = PathBuf::from(p);
                let meta = match std::fs::symlink_metadata(&pb) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let is_dir = meta.is_dir();
                let name = pb
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let parent = pb
                    .parent()
                    .map(|q| q.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let path_s = pb.to_string_lossy().replace('\\', "/");
                let entry = FileEntry {
                    path: std::sync::Arc::from(path_s.as_str()),
                    parent: std::sync::Arc::from(parent.as_str()),
                    name: std::sync::Arc::from(name.as_str()),
                    ext: std::sync::Arc::from(""),
                    size: if is_dir { 0 } else { meta.len() },
                    mtime_ms: 0,
                    btime_ms: 0,
                    is_dir,
                    is_symlink: meta.is_symlink(),
                    hidden: false,
                    system: false,
                    depth: 0,
                    id: None,
                };
                if is_dir {
                    let sub = crate::scanner::collect_recursive(&pb, false, 1);
                    entries.push(entry);
                    entries.extend(sub);
                } else {
                    entries.push(entry);
                }
            }
            run_copy(entries, opts, tx, cancel_clone);
        })
        .expect("spawn copy thread");

    CopyHandle { cancel }
}

/// Copy explicit (absolute source, relative destination) pairs into `dest`.
/// Used for the in-app paste fast path of the filter-aware clipboard, where
/// the relative structure was computed at copy time.
pub fn start_copy_pairs(
    pairs: Vec<(String, String)>,
    dest: PathBuf,
    conflict: Conflict,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let start = Instant::now();
            let files_total = pairs.len() as u64;
            let bytes_total: u64 = pairs
                .iter()
                .filter_map(|(abs, _)| std::fs::metadata(abs).ok().map(|m| m.len()))
                .sum();
            let mut files_done = 0u64;
            let mut bytes_done = 0u64;
            let mut errors_count = 0u64;
            let mut errors: Vec<(String, String)> = Vec::new();
            let mut last_progress = Instant::now();

            for (abs, rel) in &pairs {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                let mut target = dest.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if target.exists() {
                    match conflict {
                        Conflict::Skip => {
                            files_done += 1;
                            continue;
                        }
                        Conflict::Rename => target = unique_path(&target),
                        Conflict::Overwrite => {
                            let _ = std::fs::remove_file(&target);
                        }
                    }
                }
                match std::fs::copy(abs, &target) {
                    Ok(n) => {
                        files_done += 1;
                        bytes_done = bytes_done.saturating_add(n);
                    }
                    Err(e) => {
                        errors_count += 1;
                        errors.push((abs.clone(), e.to_string()));
                        files_done += 1;
                    }
                }
                if last_progress.elapsed().as_millis() > 80 {
                    let _ = tx.send(CopyMsg::Progress(CopyProgress {
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        current_path: abs.clone(),
                        errors: errors_count,
                        done: false,
                    }));
                    last_progress = Instant::now();
                }
            }

            let _ = tx.send(CopyMsg::Done {
                progress: CopyProgress {
                    files_done,
                    files_total,
                    bytes_done,
                    bytes_total,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    current_path: String::new(),
                    errors: errors_count,
                    done: true,
                },
                errors,
            });
        })
        .expect("spawn copy thread");

    CopyHandle { cancel }
}

fn run_copy(
    entries: Vec<FileEntry>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
    cancel: Arc<AtomicBool>,
) {
    let start = Instant::now();
    let root_fwd = opts.root.to_string_lossy().replace('\\', "/");
    let root_fwd = root_fwd.trim_end_matches('/').to_string();

    // Spec: only files emitted; structure built via parents. Empty selected dirs are skipped.
    let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();
    let files_total = files.len() as u64;
    let bytes_total: u64 = files.iter().map(|f| f.size).sum();

    let mut files_done: u64 = 0;
    let mut bytes_done: u64 = 0;
    let mut errors_count: u64 = 0;
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut last_progress = Instant::now();

    let send_progress = |files_done: u64,
                         bytes_done: u64,
                         current: &str,
                         errs: u64,
                         done: bool|
     -> CopyProgress {
        CopyProgress {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
            elapsed_ms: start.elapsed().as_millis() as u64,
            current_path: current.to_string(),
            errors: errs,
            done,
        }
    };

    for f in &files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let src_str = f.path.as_ref();
        let src = PathBuf::from(src_str);
        let rel = if opts.preserve_structure {
            rel_from_root(src_str, &root_fwd)
        } else {
            f.name.to_string()
        };
        let mut target = opts.dest.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));

        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                errors.push((src_str.to_string(), e.to_string()));
                errors_count += 1;
                files_done += 1;
                continue;
            }
        }

        if target.exists() {
            match opts.conflict {
                Conflict::Skip => {
                    files_done += 1;
                    continue;
                }
                Conflict::Rename => {
                    target = unique_path(&target);
                }
                Conflict::Overwrite => {
                    let _ = std::fs::remove_file(&target);
                }
            }
        }

        let result = if opts.mode == CopyMode::Move {
            match std::fs::rename(&src, &target) {
                Ok(_) => Ok(()),
                Err(e) if e.raw_os_error() == Some(17) /* EXDEV */
                       || e.kind() == std::io::ErrorKind::CrossesDevices => {
                    // Cross-volume: copy + remove
                    std::fs::copy(&src, &target).and_then(|_| std::fs::remove_file(&src)).map(|_| ())
                }
                Err(e) => Err(e),
            }
        } else {
            std::fs::copy(&src, &target).map(|_| ())
        };

        match result {
            Ok(_) => {
                files_done += 1;
                bytes_done = bytes_done.saturating_add(f.size);
            }
            Err(e) => {
                errors_count += 1;
                errors.push((src_str.to_string(), e.to_string()));
                files_done += 1;
            }
        }

        if last_progress.elapsed().as_millis() > 80 {
            let _ = tx.send(CopyMsg::Progress(send_progress(
                files_done,
                bytes_done,
                src_str,
                errors_count,
                false,
            )));
            last_progress = Instant::now();
        }
    }

    // After move, prune empty source dirs (best-effort)
    if opts.mode == CopyMode::Move && opts.preserve_structure {
        prune_empty_dirs(&opts.root, &files, &root_fwd);
    }

    let done = send_progress(files_done, bytes_done, "", errors_count, true);
    let _ = tx.send(CopyMsg::Done { progress: done, errors });
}

fn prune_empty_dirs(root: &Path, files: &[&FileEntry], root_fwd: &str) {
    let mut dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for f in files {
        let p = PathBuf::from(f.path.as_ref());
        let mut cur = p.parent().map(|p| p.to_path_buf());
        while let Some(c) = cur {
            if c.to_string_lossy().replace('\\', "/").len() <= root_fwd.len() {
                break;
            }
            dirs.insert(c.clone());
            cur = c.parent().map(|p| p.to_path_buf());
        }
        let _ = root; // suppress unused
    }
    let mut sorted: Vec<_> = dirs.into_iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
    for d in sorted {
        let _ = std::fs::remove_dir(d);
    }
}
