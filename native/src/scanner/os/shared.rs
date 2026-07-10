use super::budget::ScanBudget;
use super::core::ms_since_unix;
use super::platform::{get_attrs, is_link_like, path_text};
use super::walk::walk_parallel;
use crate::types::{FileEntry, ScanProgress};
use crossbeam_channel::Sender;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub enum ScanMessage {
    Entries(Vec<FileEntry>),
    Progress(ScanProgress),
    Error(String),
    /// One or more paths that could not be read. Sent as a batch.
    FailedPaths(Vec<(String, String)>),
    Done(ScanProgress),
}

pub struct ScanHandle {
    pub cancel: Arc<AtomicBool>,
}

const MAX_ERROR_PATHS_TRACKED: usize = 500;

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

    let failure_tx = tx.clone();
    if let Err(error) = std::thread::Builder::new()
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
    {
        cancel.store(true, Ordering::Relaxed);
        let detail = format!("scan worker could not start: {error}");
        let _ = failure_tx.send(ScanMessage::Error(detail));
        let _ = failure_tx.send(ScanMessage::Done(ScanProgress {
            scanned: 0,
            bytes: 0,
            errors: 1,
            elapsed_ms: 0,
            current_path: String::new(),
        }));
    }

    ScanHandle { cancel }
}

pub(super) struct Scanner {
    pub(super) opts: ScanOpts,
    pub(super) tx: Sender<ScanMessage>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) scanned: Arc<AtomicU64>,
    pub(super) bytes: Arc<AtomicU64>,
    pub(super) errors: Arc<AtomicU64>,
    pub(super) start: Instant,
    pub(super) sample_path: Arc<Mutex<String>>,
    /// Capped list of (path, error message) for surfacing in the UI.
    pub(super) failed_paths: Arc<Mutex<Vec<(String, String)>>>,
    pub(super) budget: ScanBudget,
    pub(super) budget_exhausted: AtomicBool,
    pub(super) visited_directories: Mutex<HashSet<String>>,
}

impl Scanner {
    pub(super) fn send(&self, message: ScanMessage) -> bool {
        if self.tx.send(message).is_ok() {
            true
        } else {
            self.cancel.store(true, Ordering::Relaxed);
            false
        }
    }

    pub(super) fn claim_entry(&self, text_bytes: u64, depth: u32, path: &str) -> bool {
        match self.budget.claim(text_bytes, depth) {
            Ok(()) => true,
            Err(limit) => {
                if !self.budget_exhausted.swap(true, Ordering::Relaxed) {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    let detail = format!(
                        "scan stopped because its bounded {limit} limit was reached at {path}"
                    );
                    record_failure(&self.failed_paths, path, detail.clone());
                    let _ = self.tx.send(ScanMessage::Error(detail));
                }
                self.cancel.store(true, Ordering::Relaxed);
                false
            }
        }
    }

    pub(super) fn enter_directory(&self, directory: &PathBuf) -> bool {
        if !self.opts.follow_symlinks {
            return true;
        }
        let canonical = match std::fs::canonicalize(directory) {
            Ok(path) => path,
            Err(error) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                record_failure(
                    &self.failed_paths,
                    &directory.to_string_lossy(),
                    format!("canonicalize: {error}"),
                );
                return false;
            }
        };
        let key = canonical.to_string_lossy().into_owned();
        #[cfg(windows)]
        let key = key.to_ascii_lowercase();
        self.visited_directories
            .lock()
            .map(|mut visited| visited.insert(key))
            .unwrap_or(false)
    }
}

pub(super) fn record_failure(failed: &Mutex<Vec<(String, String)>>, path: &str, msg: String) {
    if let Ok(mut g) = failed.lock() {
        if g.len() < MAX_ERROR_PATHS_TRACKED {
            g.push((path.to_string(), msg));
        }
    }
}

fn finish_root_failure(
    tx: &Sender<ScanMessage>,
    failed: &Mutex<Vec<(String, String)>>,
    root: &PathBuf,
    start: Instant,
    detail: String,
) {
    record_failure(failed, &format!("{root:?}"), detail.clone());
    let _ = tx.send(ScanMessage::Error(detail));
    let _ = tx.send(ScanMessage::Done(ScanProgress {
        scanned: 0,
        bytes: 0,
        errors: 1,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: String::new(),
    }));
}

fn run_scan(root: PathBuf, opts: ScanOpts, tx: Sender<ScanMessage>, cancel: Arc<AtomicBool>) {
    let start = Instant::now();
    let scanned = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let sample_path = Arc::new(Mutex::new(String::new()));
    let failed_paths: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));

    let root_text = match path_text(&root) {
        Some(path) => path,
        None => {
            finish_root_failure(
                &tx,
                &failed_paths,
                &root,
                start,
                "Wurzelpfad ist kein gültiges Unicode".to_string(),
            );
            return;
        }
    };
    let root_parent = match root.parent() {
        Some(parent) => match path_text(parent) {
            Some(parent) => parent,
            None => {
                finish_root_failure(
                    &tx,
                    &failed_paths,
                    &root,
                    start,
                    "Wurzel-Elternpfad ist kein gültiges Unicode".to_string(),
                );
                return;
            }
        },
        None => String::new(),
    };
    let root_name = match root.file_name() {
        Some(name) => match name.to_str() {
            Some(name) => name.to_string(),
            None => {
                finish_root_failure(
                    &tx,
                    &failed_paths,
                    &root,
                    start,
                    "Wurzelname ist kein gültiges Unicode".to_string(),
                );
                return;
            }
        },
        None => root_text.clone(),
    };

    // Emit root entry — always, regardless of hidden/system. The view filter
    // is responsible for hiding entries the user doesn't want to see.
    let root_link_like = match std::fs::symlink_metadata(&root) {
        Ok(meta) => {
            let root_link_like = is_link_like(&meta);
            let (hidden, system) = get_attrs(&meta);
            let entry = FileEntry {
                path: Arc::from(root_text.as_str()),
                parent: Arc::from(root_parent.as_str()),
                name: Arc::from(root_name.as_str()),
                ext: Arc::from(""),
                size: 0,
                mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
                btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
                is_dir: true,
                is_symlink: root_link_like,
                hidden,
                system,
                depth: 0,
                id: None,
            };
            if tx.send(ScanMessage::Entries(vec![entry])).is_err() {
                cancel.store(true, Ordering::Relaxed);
                return;
            }
            root_link_like
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
            }));
            return;
        }
    };

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
        budget: ScanBudget::default(),
        budget_exhausted: AtomicBool::new(false),
        visited_directories: Mutex::new(HashSet::new()),
    });

    // Periodic progress emitter
    let progress_thread = {
        let s = scanner.clone();
        let cancel_p = cancel.clone();
        std::thread::spawn(move || {
            while !cancel_p.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                let cur = s.sample_path.lock().map(|x| x.clone()).unwrap_or_default();
                if !s.send(ScanMessage::Progress(ScanProgress {
                    scanned: s.scanned.load(Ordering::Relaxed),
                    bytes: s.bytes.load(Ordering::Relaxed),
                    errors: s.errors.load(Ordering::Relaxed),
                    elapsed_ms: s.start.elapsed().as_millis() as u64,
                    current_path: cur,
                })) {
                    break;
                }
            }
        })
    };

    // Walk
    if !root_link_like || scanner.opts.follow_symlinks {
        walk_parallel(&scanner, vec![root.clone()], 1);
    }

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
    };
    let _ = tx.send(ScanMessage::Done(final_progress));
}
