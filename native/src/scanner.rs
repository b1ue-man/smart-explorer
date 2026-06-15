use crate::types::{FileEntry, ScanProgress};
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
#[cfg(windows)]
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

const BATCH_SIZE: usize = 1024;
const FLUSH_INTERVAL_MS: u128 = 60;

pub enum ScanMessage {
    Entries(Vec<FileEntry>),
    Progress(ScanProgress),
    Error(String),
    /// One or more paths that could not be read. Sent as a batch.
    FailedPaths(Vec<(String, String)>),
    Done(ScanProgress),
}

const MAX_ERROR_PATHS_TRACKED: usize = 500;

pub struct ScanHandle {
    pub cancel: Arc<AtomicBool>,
}

#[inline]
fn ms_since_unix(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

#[inline]
fn ext_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

#[cfg(windows)]
fn get_attrs(meta: &std::fs::Metadata) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;
    let a = meta.file_attributes();
    (
        a & FILE_ATTRIBUTE_HIDDEN != 0,
        a & FILE_ATTRIBUTE_SYSTEM != 0,
    )
}

#[cfg(not(windows))]
fn get_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

pub struct ScanOpts {
    pub follow_symlinks: bool,
    /// Maximum depth to descend. `Some(1)` = current dir only (Explorer-style).
    /// `None` = unlimited recursion.
    pub max_depth: Option<u32>,
}

pub fn start_scan(
    root: PathBuf,
    follow_symlinks: bool,
    max_depth: Option<u32>,
    tx: Sender<ScanMessage>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::Builder::new()
        .name("scan-driver".into())
        .spawn(move || {
            run_scan(
                root,
                ScanOpts {
                    follow_symlinks,
                    max_depth,
                },
                tx,
                cancel_clone,
            );
        })
        .expect("spawn scan thread");

    ScanHandle { cancel }
}

struct Scanner {
    opts: ScanOpts,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    start: Instant,
    sample_path: Arc<Mutex<String>>,
    /// Capped list of (path, error message) for surfacing in the UI.
    failed_paths: Arc<Mutex<Vec<(String, String)>>>,
}

fn record_failure(failed: &Mutex<Vec<(String, String)>>, path: &str, msg: String) {
    if let Ok(mut g) = failed.lock() {
        if g.len() < MAX_ERROR_PATHS_TRACKED {
            g.push((path.to_string(), msg));
        }
    }
}

fn run_scan(
    root: PathBuf,
    opts: ScanOpts,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    let start = Instant::now();
    let scanned = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let sample_path = Arc::new(Mutex::new(String::new()));
    let failed_paths: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));

    // Emit root entry — always, regardless of hidden/system. The view filter
    // is responsible for hiding entries the user doesn't want to see.
    match std::fs::symlink_metadata(&root) {
        Ok(meta) => {
            let (hidden, system) = get_attrs(&meta);
            let parent = root
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string());
            let path_s = root.to_string_lossy().replace('\\', "/");
            let entry = FileEntry {
                path: Arc::from(path_s.as_str()),
                parent: Arc::from(parent.as_str()),
                name: Arc::from(name.as_str()),
                ext: Arc::from(""),
                size: 0,
                mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
                btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
                is_dir: true,
                is_symlink: meta.is_symlink(),
                hidden,
                system,
                depth: 0,
            };
            let _ = tx.send(ScanMessage::Entries(vec![entry]));
        }
        Err(e) => {
            record_failure(&failed_paths, &root.to_string_lossy(), e.to_string());
            let _ = tx.send(ScanMessage::Error(format!(
                "Wurzel kann nicht gelesen werden: {} ({})",
                root.display(),
                e
            )));
            let _ = tx.send(ScanMessage::Done(ScanProgress {
                scanned: 0,
                bytes: 0,
                errors: 1,
                elapsed_ms: start.elapsed().as_millis() as u64,
                current_path: String::new(),
                done: true,
            }));
            return;
        }
    }

    let scanner = Arc::new(Scanner {
        opts,
        tx: tx.clone(),
        cancel: cancel.clone(),
        scanned,
        bytes,
        errors,
        start,
        sample_path: sample_path.clone(),
        failed_paths: failed_paths.clone(),
    });

    // Periodic progress emitter
    let progress_thread = {
        let s = scanner.clone();
        let cancel_p = cancel.clone();
        std::thread::spawn(move || {
            while !cancel_p.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                let cur = s.sample_path.lock().map(|x| x.clone()).unwrap_or_default();
                let _ = s.tx.send(ScanMessage::Progress(ScanProgress {
                    scanned: s.scanned.load(Ordering::Relaxed),
                    bytes: s.bytes.load(Ordering::Relaxed),
                    errors: s.errors.load(Ordering::Relaxed),
                    elapsed_ms: s.start.elapsed().as_millis() as u64,
                    current_path: cur,
                    done: false,
                }));
            }
        })
    };

    // Walk
    walk_parallel(&scanner, vec![root.clone()], 1);

    // Stop progress thread
    cancel.store(true, Ordering::Relaxed);
    let _ = progress_thread.join();

    // Emit collected failed paths (capped)
    if let Ok(g) = failed_paths.lock() {
        if !g.is_empty() {
            let _ = tx.send(ScanMessage::FailedPaths(g.clone()));
        }
    }

    let final_progress = ScanProgress {
        scanned: scanner.scanned.load(Ordering::Relaxed),
        bytes: scanner.bytes.load(Ordering::Relaxed),
        errors: scanner.errors.load(Ordering::Relaxed),
        elapsed_ms: scanner.start.elapsed().as_millis() as u64,
        current_path: String::new(),
        done: true,
    };
    let _ = tx.send(ScanMessage::Done(final_progress));
}

/// Walks dirs in parallel using rayon. Each directory's entries are read with
/// `std::fs::read_dir`, which on Windows uses FindFirstFileW and returns full
/// metadata pre-cached on the DirEntry — so `entry.metadata()` requires no
/// additional syscall. Subdirectories are recursed via rayon's join, which
/// gives work-stealing parallelism across cores.
fn walk_parallel(scanner: &Arc<Scanner>, dirs: Vec<PathBuf>, depth: u32) {
    use rayon::prelude::*;

    if dirs.is_empty() {
        return;
    }
    if scanner.cancel.load(Ordering::Relaxed) {
        return;
    }

    dirs.into_par_iter().for_each(|dir| {
        if scanner.cancel.load(Ordering::Relaxed) {
            return;
        }
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                scanner.errors.fetch_add(1, Ordering::Relaxed);
                record_failure(
                    &scanner.failed_paths,
                    &dir.to_string_lossy(),
                    format!("read_dir: {}", e),
                );
                return;
            }
        };

        let mut batch: Vec<FileEntry> = Vec::with_capacity(64);
        let mut subdirs: Vec<PathBuf> = Vec::with_capacity(16);
        let mut last_flush = Instant::now();

        // Intern the parent path once per directory: every entry in this
        // directory shares the same `parent`, so cloning the Arc is much
        // cheaper than allocating a new Arc<str> per entry.
        let parent_str = dir.to_string_lossy().replace('\\', "/");
        let parent_arc: Arc<str> = Arc::from(parent_str.as_str());

        // Sample current dir occasionally
        if let Ok(mut sp) = scanner.sample_path.try_lock() {
            *sp = dir.to_string_lossy().to_string();
        }

        for entry_result in read {
            if scanner.cancel.load(Ordering::Relaxed) {
                break;
            }
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    scanner.errors.fetch_add(1, Ordering::Relaxed);
                    record_failure(
                        &scanner.failed_paths,
                        &dir.to_string_lossy(),
                        format!("read_dir entry: {}", e),
                    );
                    continue;
                }
            };
            // Fall back to symlink_metadata if entry.metadata() fails — this
            // recovers some entries whose target can't be resolved (broken
            // symlinks, reparse points the user can't traverse).
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => match std::fs::symlink_metadata(entry.path()) {
                    Ok(m) => m,
                    Err(e) => {
                        scanner.errors.fetch_add(1, Ordering::Relaxed);
                        record_failure(
                            &scanner.failed_paths,
                            &entry.path().to_string_lossy(),
                            format!("metadata: {}", e),
                        );
                        continue;
                    }
                },
            };

            let (hidden, system) = get_attrs(&meta);
            // No scan-time hidden/system filtering — the view filter decides
            // whether to display them. This guarantees the scanner emits a
            // complete listing.

            let is_dir = meta.is_dir();
            let is_symlink = meta.is_symlink();
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let ext = ext_of(&name, is_dir);
            let path_s = path.to_string_lossy().replace('\\', "/");
            let size = if is_dir { 0 } else { meta.len() };

            let fe = FileEntry {
                path: Arc::from(path_s.as_str()),
                parent: parent_arc.clone(),
                name: Arc::from(name.as_str()),
                ext: Arc::from(ext.as_str()),
                size,
                mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
                btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
                is_dir,
                is_symlink,
                hidden,
                system,
                depth,
            };

            scanner.scanned.fetch_add(1, Ordering::Relaxed);
            if !is_dir {
                scanner.bytes.fetch_add(size, Ordering::Relaxed);
            }
            batch.push(fe);

            if is_dir && (!is_symlink || scanner.opts.follow_symlinks) {
                let within_depth = match scanner.opts.max_depth {
                    Some(max) => depth + 1 <= max,
                    None => true,
                };
                if within_depth {
                    subdirs.push(path);
                }
            }

            if batch.len() >= BATCH_SIZE || last_flush.elapsed().as_millis() > FLUSH_INTERVAL_MS {
                let chunk = std::mem::replace(&mut batch, Vec::with_capacity(64));
                let _ = scanner.tx.send(ScanMessage::Entries(chunk));
                last_flush = Instant::now();
            }
        }

        if !batch.is_empty() {
            let _ = scanner.tx.send(ScanMessage::Entries(batch));
        }

        // Recurse into subdirs in parallel
        if !subdirs.is_empty() {
            walk_parallel(scanner, subdirs, depth + 1);
        }
    });
}

/// Synchronous recursive walk that returns all entries beneath `root` (excluding
/// the root itself). Used to expand a folder selection during a filtered copy
/// operation without going through the channel/UI plumbing.
pub fn collect_recursive(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
) -> Vec<FileEntry> {
    let result = std::sync::Mutex::new(Vec::<FileEntry>::with_capacity(1024));
    let opts = ScanOpts {
        follow_symlinks,
        max_depth: None,
    };
    walk_into_vec(&result, &opts, vec![root.to_path_buf()], start_depth);
    result.into_inner().unwrap_or_default()
}

fn walk_into_vec(
    out: &std::sync::Mutex<Vec<FileEntry>>,
    opts: &ScanOpts,
    dirs: Vec<PathBuf>,
    depth: u32,
) {
    use rayon::prelude::*;

    if dirs.is_empty() {
        return;
    }

    dirs.into_par_iter().for_each(|dir| {
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut local: Vec<FileEntry> = Vec::with_capacity(64);
        let mut subdirs: Vec<PathBuf> = Vec::new();
        let parent_str = dir.to_string_lossy().replace('\\', "/");
        let parent_arc: Arc<str> = Arc::from(parent_str.as_str());
        for er in read {
            let entry = match er {
                Ok(e) => e,
                Err(_) => continue,
            };
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => match std::fs::symlink_metadata(entry.path()) {
                    Ok(m) => m,
                    Err(_) => continue,
                },
            };
            let (hidden, system) = get_attrs(&meta);
            // No filtering — copy expansion is purely structural; the caller
            // applies the user filter afterwards.
            let is_dir = meta.is_dir();
            let is_symlink = meta.is_symlink();
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let ext = ext_of(&name, is_dir);
            let path_s = path.to_string_lossy().replace('\\', "/");
            let size = if is_dir { 0 } else { meta.len() };
            local.push(FileEntry {
                path: Arc::from(path_s.as_str()),
                parent: parent_arc.clone(),
                name: Arc::from(name.as_str()),
                ext: Arc::from(ext.as_str()),
                size,
                mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
                btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
                is_dir,
                is_symlink,
                hidden,
                system,
                depth,
            });
            if is_dir && (!is_symlink || opts.follow_symlinks) {
                subdirs.push(path);
            }
        }
        if !local.is_empty() {
            if let Ok(mut g) = out.lock() {
                g.extend(local);
            }
        }
        if !subdirs.is_empty() {
            walk_into_vec(out, opts, subdirs, depth + 1);
        }
    });
}
