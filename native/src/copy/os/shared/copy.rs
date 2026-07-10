use crate::types::{Conflict, CopyMode, CopyOptions, CopyProgress, FileEntry};
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::platform;

#[path = "durability.rs"]
mod durability;
#[path = "move_guard.rs"]
mod move_guard;
#[path = "outcome.rs"]
mod outcome;
#[path = "path_guard.rs"]
mod path_guard;
#[path = "planning.rs"]
mod planning;
#[path = "prune.rs"]
mod prune;
#[path = "relative.rs"]
mod relative;
#[path = "safe_file.rs"]
mod safe_file;
use outcome::{send_collection_failure, send_copy_canceled, CopyErrorLog};
use planning::{dedupe_entries, dedupe_paths, EntryAccumulator};
use prune::{prune_empty_dirs, selected_directory_roots};
use relative::{rel_from_root, safe_rel_path, validate_seed_destinations};
use safe_file::{transfer_file, TransferResult};

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

fn send_copy_failure(tx: &Sender<CopyMsg>, path: String, detail: String) {
    let _ = tx.send(CopyMsg::Done {
        progress: CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            errors: 1,
            canceled: false,
            done: true,
        },
        errors: vec![(path, detail)],
    });
}

/// Copy selected entries. Directory expansion (recursive walk of selected
/// folders) happens on the worker thread, so the UI never blocks on a large
/// subtree. `filter` (with its root prefix) is applied to the expanded entries;
/// selected plain files always pass.
pub fn start_copy_expanded(
    seeds: Vec<FileEntry>,
    filter: Option<(crate::types::FilterDef, String)>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let failure_tx = tx.clone();

    let spawn = std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let seeds = dedupe_entries(seeds);
            if let Err(error) = validate_seed_destinations(&seeds, &opts) {
                send_copy_failure(&tx, opts.dest.display().to_string(), error);
                return;
            }
            let move_roots = selected_directory_roots(&seeds);
            let cf = filter
                .as_ref()
                .map(|(f, prefix)| (crate::filter::CompiledFilter::compile(f), prefix.clone()));
            let mut entries = EntryAccumulator::default();
            for e in &seeds {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                if e.is_dir && !e.is_symlink {
                    let collected = crate::scanner::collect_recursive(
                        &PathBuf::from(e.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                        false,
                        e.depth + 1,
                        &cancel_clone,
                    );
                    if collected.canceled {
                        send_copy_canceled(&tx);
                        return;
                    }
                    if !collected.is_complete() {
                        send_collection_failure(&tx, collected);
                        return;
                    }
                    if let Err(error) = entries.push(e.clone()) {
                        send_copy_failure(&tx, e.path.to_string(), error);
                        return;
                    }
                    let append = match &cf {
                        Some((cf, prefix)) => entries.extend(
                            collected
                                .entries
                                .into_iter()
                                .filter(|entry| cf.matches(entry, prefix)),
                        ),
                        None => entries.extend(collected.entries),
                    };
                    if let Err(error) = append {
                        send_copy_failure(&tx, e.path.to_string(), error);
                        return;
                    }
                } else if let Err(error) = entries.push(e.clone()) {
                    send_copy_failure(&tx, e.path.to_string(), error);
                    return;
                }
            }
            run_copy(entries.into_entries(), opts, tx, cancel_clone, move_roots);
        });
    if let Err(error) = spawn {
        send_copy_failure(&failure_tx, "Kopieren".to_string(), error.to_string());
    }

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
    let failure_tx = tx.clone();

    let spawn = std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let paths = dedupe_paths(paths);
            let mut entries = EntryAccumulator::default();
            let mut move_roots = Vec::new();
            for p in &paths {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                let pb = PathBuf::from(p);
                let meta = match std::fs::symlink_metadata(&pb) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        send_copy_failure(&tx, p.clone(), error.to_string());
                        return;
                    }
                };
                let is_dir = meta.is_dir();
                let name = match pb.file_name().and_then(|name| name.to_str()) {
                    Some(name) => name.to_string(),
                    None => {
                        send_copy_failure(
                            &tx,
                            p.clone(),
                            "Dateiname ist kein gültiges Unicode".to_string(),
                        );
                        return;
                    }
                };
                let parent = match pb.parent().map(platform::path_text) {
                    Some(Ok(parent)) => parent,
                    _ => {
                        send_copy_failure(
                            &tx,
                            p.clone(),
                            "Quellordner ist kein gültiges Unicode".to_string(),
                        );
                        return;
                    }
                };
                let path_s = match platform::path_text(&pb) {
                    Ok(path) => path,
                    Err(_) => {
                        send_copy_failure(
                            &tx,
                            p.clone(),
                            "Quellpfad ist kein gültiges Unicode".to_string(),
                        );
                        return;
                    }
                };
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
                    if let Err(error) =
                        validate_seed_destinations(std::slice::from_ref(&entry), &opts)
                    {
                        send_copy_failure(&tx, p.clone(), error);
                        return;
                    }
                    let collected = crate::scanner::collect_recursive(&pb, false, 1, &cancel_clone);
                    if collected.canceled {
                        send_copy_canceled(&tx);
                        return;
                    }
                    if !collected.is_complete() {
                        send_collection_failure(&tx, collected);
                        return;
                    }
                    move_roots.extend(selected_directory_roots(std::slice::from_ref(&entry)));
                    if let Err(error) = entries.push(entry) {
                        send_copy_failure(&tx, p.clone(), error);
                        return;
                    }
                    if let Err(error) = entries.extend(collected.entries) {
                        send_copy_failure(&tx, p.clone(), error);
                        return;
                    }
                } else if let Err(error) = entries.push(entry) {
                    send_copy_failure(&tx, p.clone(), error);
                    return;
                }
            }
            run_copy(entries.into_entries(), opts, tx, cancel_clone, move_roots);
        });
    if let Err(error) = spawn {
        send_copy_failure(&failure_tx, "Kopieren".to_string(), error.to_string());
    }

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
    let failure_tx = tx.clone();

    let spawn = std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let start = Instant::now();
            if let Err(error) = planning::validate_pair_budget(&pairs) {
                send_copy_failure(&tx, dest.display().to_string(), error);
                return;
            }
            let files_total = pairs.len() as u64;
            let bytes_total: u64 = pairs
                .iter()
                .filter_map(|(abs, _)| std::fs::metadata(abs).ok().map(|m| m.len()))
                .sum();
            let mut files_done = 0u64;
            let mut bytes_done = 0u64;
            let mut errors = CopyErrorLog::default();
            let mut last_progress = Instant::now();

            for (abs, rel) in &pairs {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                let Some(rel_path) = safe_rel_path(rel) else {
                    errors.record(abs.clone(), "ungueltiger relativer Zielpfad".to_string());
                    files_done += 1;
                    continue;
                };
                let target = dest.join(rel_path);
                match transfer_file(
                    Path::new(abs),
                    &target,
                    &dest,
                    conflict,
                    CopyMode::Copy,
                    &cancel_clone,
                ) {
                    Ok(TransferResult::Completed(n)) => {
                        files_done += 1;
                        bytes_done = bytes_done.saturating_add(n);
                    }
                    Ok(TransferResult::Skipped) => files_done += 1,
                    Ok(TransferResult::Canceled) => break,
                    Err(e) => {
                        errors.record(abs.clone(), e.to_string());
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
                        errors: errors.total(),
                        canceled: false,
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
                    errors: errors.total(),
                    canceled: cancel_clone.load(Ordering::Relaxed),
                    done: true,
                },
                errors: errors.into_items(),
            });
        });
    if let Err(error) = spawn {
        send_copy_failure(&failure_tx, "Kopieren".to_string(), error.to_string());
    }

    CopyHandle { cancel }
}

fn run_copy(
    entries: Vec<FileEntry>,
    opts: CopyOptions,
    tx: Sender<CopyMsg>,
    cancel: Arc<AtomicBool>,
    move_roots: Vec<PathBuf>,
) {
    let start = Instant::now();
    let root_fwd = match platform::path_text(&opts.root) {
        Ok(root) => root,
        Err(error) => {
            send_copy_failure(&tx, opts.root.display().to_string(), error.to_string());
            return;
        }
    };
    let root_fwd = root_fwd.trim_end_matches('/').to_string();

    for entry in &entries {
        if entry.is_dir && !entry.is_symlink {
            continue;
        }
        let source = PathBuf::from(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let metadata = match std::fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) => {
                send_copy_failure(&tx, entry.path.to_string(), error.to_string());
                return;
            }
        };
        if entry.is_symlink
            || platform::metadata_is_link_like(&metadata)
            || (!entry.is_dir && !metadata.is_file())
        {
            send_copy_failure(
                &tx,
                entry.path.to_string(),
                "Links, Reparse-Punkte und Spezialdateien werden nicht automatisch übertragen."
                    .to_string(),
            );
            return;
        }
    }

    // Spec: only files emitted; structure built via parents. Empty selected dirs are skipped.
    let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();
    let files_total = files.len() as u64;
    let bytes_total: u64 = files.iter().map(|f| f.size).sum();

    let mut files_done: u64 = 0;
    let mut bytes_done: u64 = 0;
    let mut errors = CopyErrorLog::default();
    let mut last_progress = Instant::now();

    let send_progress = |files_done: u64, bytes_done: u64, errs: u64, done: bool| -> CopyProgress {
        CopyProgress {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
            elapsed_ms: start.elapsed().as_millis() as u64,
            errors: errs,
            canceled: done && cancel.load(Ordering::Relaxed),
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
        let Some(rel_path) = safe_rel_path(&rel) else {
            errors.record(
                src_str.to_string(),
                "ungueltiger relativer Zielpfad".to_string(),
            );
            files_done += 1;
            continue;
        };
        let target = opts.dest.join(rel_path);
        match transfer_file(&src, &target, &opts.dest, opts.conflict, opts.mode, &cancel) {
            Ok(TransferResult::Completed(bytes)) => {
                files_done += 1;
                bytes_done = bytes_done.saturating_add(bytes);
            }
            Ok(TransferResult::Skipped) => files_done += 1,
            Ok(TransferResult::Canceled) => break,
            Err(e) => {
                errors.record(src_str.to_string(), e.to_string());
                files_done += 1;
            }
        }

        if last_progress.elapsed().as_millis() > 80 {
            let _ = tx.send(CopyMsg::Progress(send_progress(
                files_done,
                bytes_done,
                errors.total(),
                false,
            )));
            last_progress = Instant::now();
        }
    }

    // After move, prune empty source dirs (best-effort)
    if opts.mode == CopyMode::Move {
        for (path, detail) in prune_empty_dirs(&move_roots, &entries) {
            errors.record(path, detail);
        }
    }

    let done = send_progress(files_done, bytes_done, errors.total(), true);
    let _ = tx.send(CopyMsg::Done {
        progress: done,
        errors: errors.into_items(),
    });
}
