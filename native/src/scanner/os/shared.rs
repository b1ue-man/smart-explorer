use super::budget::ScanBudget;
use super::core::ms_since_unix;
use super::platform::{get_attrs, is_link_like, path_text};
use super::retention::RetentionHandle;
use super::walk::{scan_threads, walk_parallel, PendingDir};
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
    DeniedRoots(super::DeniedRoots),
    Done(ScanProgress),
}

pub struct ScanHandle {
    pub cancel: Arc<AtomicBool>,
    /// Set when the bounded scan budget (entries, retained text, depth)
    /// stopped the walk before it visited everything.
    pub truncated: Arc<AtomicBool>,
}

#[cfg(test)]
#[path = "search_recursive_access_task.rs"]
mod task;

#[cfg(test)]
#[path = "failed_paths_tests.rs"]
mod failed_paths_tests;

pub struct ScanOpts {
    pub follow_symlinks: bool,
    /// Maximum depth to descend. `Some(1)` = current dir only (Explorer-style).
    /// `None` = unlimited recursion.
    pub max_depth: Option<u32>,
    /// Scan-time retention: when set, only retained entries and the
    /// directories needed to place them are emitted and counted against the
    /// budget; `None` emits every visited entry.
    pub retention: Option<RetentionHandle>,
}

impl ScanOpts {
    /// Emit everything down to `max_depth` (`None` = unlimited).
    pub fn everything(max_depth: Option<u32>) -> Self {
        Self {
            follow_symlinks: false,
            max_depth,
            retention: None,
        }
    }
}

pub fn start_scan(root: PathBuf, opts: ScanOpts, tx: Sender<ScanMessage>) -> ScanHandle {
    start_scan_inner(root, opts, tx, None)
}

pub(crate) fn start_scan_confined(
    root: PathBuf,
    mut opts: ScanOpts,
    tx: Sender<ScanMessage>,
    handle: crate::local_access::DirectoryHandle,
) -> ScanHandle {
    opts.follow_symlinks = false;
    start_scan_inner(root, opts, tx, Some(handle))
}

fn start_scan_inner(
    root: PathBuf,
    opts: ScanOpts,
    tx: Sender<ScanMessage>,
    provided: Option<crate::local_access::DirectoryHandle>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let truncated = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let truncated_clone = truncated.clone();

    let failure_tx = tx.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("scan-driver".into())
        .spawn(move || {
            run_scan(root, opts, tx, cancel_clone, truncated_clone, provided);
        })
    {
        cancel.store(true, Ordering::Relaxed);
        let detail = format!("scan worker could not start: {error}");
        let _ = failure_tx.send(ScanMessage::Error(detail));
        let _ = failure_tx.send(ScanMessage::Done(ScanProgress {
            scanned: 0,
            bytes: 0,
            errors: 1,
            permission_denied: 0,
            elapsed_ms: 0,
            current_path: String::new(),
        }));
    }

    ScanHandle { cancel, truncated }
}

pub(super) struct Scanner {
    pub(super) opts: ScanOpts,
    pub(super) tx: Sender<ScanMessage>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) scanned: Arc<AtomicU64>,
    pub(super) bytes: Arc<AtomicU64>,
    pub(super) errors: Arc<AtomicU64>,
    pub(super) permission_denied: AtomicU64,
    pub(super) denied_roots: super::DeniedRoots,
    pub(super) start: Instant,
    pub(super) sample_path: Arc<Mutex<String>>,
    /// Capped list of (path, error message) for surfacing in the UI.
    pub(super) failed_paths: Arc<Mutex<Vec<(String, String)>>>,
    pub(super) budget: ScanBudget,
    pub(super) budget_exhausted: AtomicBool,
    pub(super) truncated: Arc<AtomicBool>,
    pub(super) visited_directories: Mutex<HashSet<String>>,
    pub(super) confined: Option<(PathBuf, crate::local_access::DirectoryHandle)>,
    pub(super) optical: Option<(PathBuf, crate::source_media::Source)>,
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
                self.truncated.store(true, Ordering::Relaxed);
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

    pub(super) fn media_current(&self) -> bool {
        if let Some((root, expected)) = &self.optical {
            let unchanged = crate::source_media::probe(root)
                .is_ok_and(|source| source.as_ref() == Some(expected));
            if !unchanged {
                self.errors.fetch_add(1, Ordering::Relaxed);
                record_failure(
                    &self.failed_paths,
                    &root.to_string_lossy(),
                    "optical medium changed during scan".into(),
                );
                self.cancel.store(true, Ordering::Release);
                return false;
            }
        }
        true
    }

    pub(super) fn enter_directory(&self, directory: &PathBuf) -> bool {
        if !self.media_current() {
            return false;
        }
        if !self.opts.follow_symlinks {
            return true;
        }
        let canonical = match crate::source_media::acquire(
            directory,
            crate::source_media::Priority::Foreground,
            &self.cancel,
        )
        .and_then(|_guard| std::fs::canonicalize(directory))
        {
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

/// Records every failed path (R15c: no count; the former 500 dropped
/// findings without a technical cause). A path recorded twice is sent once
/// (`distinct_failures`).
pub(super) fn record_failure(failed: &Mutex<Vec<(String, String)>>, path: &str, msg: String) {
    if let Ok(mut g) = failed.lock() {
        g.push((path.to_string(), msg));
    }
}

/// Each failed path once, with its first message, in the order recorded.
fn distinct_failures(failed: &[(String, String)]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    failed
        .iter()
        .filter(|(path, _)| seen.insert(path.as_str()))
        .cloned()
        .collect()
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
        permission_denied: 0,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: String::new(),
    }));
}

fn run_scan(
    root: PathBuf,
    opts: ScanOpts,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
    truncated: Arc<AtomicBool>,
    provided: Option<crate::local_access::DirectoryHandle>,
) {
    let root = crate::local_access::normalize_scan_root(&root);
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
    let mut optical = None;
    let metadata =
        crate::source_media::acquire(&root, crate::source_media::Priority::Foreground, &cancel)
            .and_then(|guard| {
                optical = guard
                    .as_ref()
                    .map(|guard| (root.clone(), guard.source().clone()));
                provided.as_ref().map_or_else(
                    || crate::local_access::symlink_metadata(&root),
                    |handle| handle.metadata(),
                )
            });
    let _root_link_like = match metadata {
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
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                let denied = super::DeniedRoots::default();
                denied.record(root_text.clone());
                denied.record_visits(root_text.clone(), super::ScanVisits::default());
                let _ = tx.send(ScanMessage::DeniedRoots(denied));
            }
            let _ = tx.send(ScanMessage::Error(format!(
                "Wurzel kann nicht gelesen werden: {} ({})",
                root.display(),
                e
            )));
            let _ = tx.send(ScanMessage::FailedPaths(vec![(
                root_text.clone(),
                e.to_string(),
            )]));
            let _ = tx.send(ScanMessage::Done(ScanProgress {
                scanned: 0,
                bytes: 0,
                errors: 1,
                permission_denied: u64::from(e.kind() == std::io::ErrorKind::PermissionDenied),
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
        permission_denied: AtomicU64::new(0),
        denied_roots: super::DeniedRoots::default(),
        start,
        sample_path: sample_path.clone(),
        failed_paths: failed_paths.clone(),
        budget: ScanBudget::default(),
        budget_exhausted: AtomicBool::new(false),
        truncated,
        visited_directories: Mutex::new(HashSet::new()),
        confined: provided.map(|handle| (root.clone(), handle)),
        optical,
    });

    // Periodic progress emitter
    let progress_thread = {
        let s = scanner.clone();
        let cancel_p = cancel.clone();
        std::thread::Builder::new()
            .name("scan-progress".into())
            .spawn(move || {
                while !cancel_p.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    let cur = s.sample_path.lock().map(|x| x.clone()).unwrap_or_default();
                    if !s.send(ScanMessage::Progress(ScanProgress {
                        scanned: s.scanned.load(Ordering::Relaxed),
                        bytes: s.bytes.load(Ordering::Relaxed),
                        errors: s.errors.load(Ordering::Relaxed),
                        permission_denied: s.permission_denied.load(Ordering::Relaxed),
                        elapsed_ms: s.start.elapsed().as_millis() as u64,
                        current_path: cur,
                    })) {
                        break;
                    }
                }
            })
    };

    // Walk. The root entry is already emitted, so its children start with no
    // pending ancestors.
    let walk = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let root_dir = PendingDir {
            path: root.clone(),
            lineage: None,
            parent: None,
        };
        // The explicitly chosen root may itself be a junction; child links
        // remain non-recursive boundaries. Reserve stack space for deep trees.
        let threads = scan_threads();
        match rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .stack_size(8 * 1024 * 1024)
            .build()
        {
            Ok(pool) => pool.install(|| walk_parallel(&scanner, vec![root_dir], 1)),
            Err(error) => {
                scanner.errors.fetch_add(1, Ordering::Relaxed);
                record_failure(&failed_paths, &root_text, format!("Scan-Threads: {error}"));
                None
            }
        }
    }));
    if let Ok(Some(visits)) = &walk {
        scanner
            .denied_roots
            .record_scan_visits(root_text.clone(), *visits);
    }
    if walk.is_err() {
        scanner.errors.fetch_add(1, Ordering::Relaxed);
        record_failure(
            &failed_paths,
            &root_text,
            "Interner Lesefehler; geladene Ergebnisse bleiben erhalten".into(),
        );
    }

    // Stop progress thread
    cancel.store(true, Ordering::Relaxed);
    if let Ok(progress_thread) = progress_thread {
        let _ = progress_thread.join();
    }

    // Emit every collected failed path once.
    if let Ok(g) = failed_paths.lock() {
        if !g.is_empty() {
            let _ = tx.send(ScanMessage::FailedPaths(distinct_failures(&g)));
        }
    }

    let final_progress = ScanProgress {
        scanned: scanner.scanned.load(Ordering::Relaxed),
        bytes: scanner.bytes.load(Ordering::Relaxed),
        errors: scanner.errors.load(Ordering::Relaxed),
        permission_denied: scanner.permission_denied.load(Ordering::Relaxed),
        elapsed_ms: scanner.start.elapsed().as_millis() as u64,
        current_path: String::new(),
    };
    let _ = tx.send(ScanMessage::DeniedRoots(scanner.denied_roots.clone()));
    let _ = tx.send(ScanMessage::Done(final_progress));
}
