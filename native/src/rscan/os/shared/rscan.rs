//! Backend-driven directory scanning for remote roots. Directory enumeration
//! goes through `vfs::Backend`; results use the same bounded `ScanMessage`
//! stream as the local scanner.

#[path = "budget.rs"]
mod budget;
#[path = "parallel.rs"]
mod parallel;
#[path = "search.rs"]
mod search;
#[path = "walk_state.rs"]
mod walk_state;

pub use search::start_search_backend;

use crate::scanner::{ScanHandle, ScanMessage};
use crate::types::{FileEntry, ScanProgress};
use crate::vfs::BackendHandle;
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use walk_state::WalkState;

const BATCH: usize = 256;
const PROGRESS_MS: u128 = 150;
const MAX_ERRORS_TRACKED: usize = 500;

fn ext_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(index) if index + 1 < name.len() && index > 0 => name[index + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn join(directory: &str, name: &str) -> String {
    if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

/// Walk `root` through `backend`, streaming results over `tx`. `Some(1)` is a
/// flat listing; `None` recursively scans within the global safety limits.
pub fn start_scan_backend(
    backend: BackendHandle,
    root: String,
    max_depth: Option<u32>,
    tx: Sender<ScanMessage>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let failure_tx = tx.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("rscan-driver".into())
        .spawn(move || run(backend, root, max_depth, tx, worker_cancel))
    {
        report_spawn_failure(&failure_tx, &cancel, "remote scan", error);
    }
    ScanHandle { cancel }
}

fn run(
    backend: BackendHandle,
    root: String,
    max_depth: Option<u32>,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    let start = Instant::now();
    let mut state = WalkState::new(tx, cancel, start);

    match backend.stat(&root) {
        Ok(metadata) => {
            let parent = match root.rsplit_once('/') {
                Some((parent, _)) if !parent.is_empty() => parent.to_string(),
                _ => String::new(),
            };
            let name = if metadata.name.is_empty() {
                root.clone()
            } else {
                metadata.name.clone()
            };
            let root_entry = FileEntry {
                path: Arc::from(root.as_str()),
                parent: Arc::from(parent.as_str()),
                name: Arc::from(name.as_str()),
                ext: Arc::from(""),
                size: 0,
                mtime_ms: metadata.mtime_ms,
                btime_ms: metadata.btime_ms,
                is_dir: true,
                is_symlink: metadata.is_symlink,
                hidden: metadata.hidden,
                system: metadata.system,
                depth: 0,
                id: None,
            };
            if !state.send_root(root_entry) {
                return;
            }
        }
        Err(error) => {
            state.terminal_error(
                &root,
                format!("Wurzel kann nicht gelesen werden: {root} ({error})"),
            );
            state.finish();
            return;
        }
    }

    if max_depth.is_none() && backend.parallelism() > 1 {
        parallel::run(backend, root, state);
        return;
    }

    let mut queue = VecDeque::from([(root, 1u32)]);
    while let Some((directory, depth)) = queue.pop_front() {
        if state.stopped() {
            break;
        }
        match backend.list_dir(&directory) {
            Ok(entries) => {
                let descend = max_depth.is_none_or(|maximum| depth < maximum);
                let mut next = Vec::new();
                if !state.process_listing(&directory, depth, entries, descend, &mut next) {
                    break;
                }
                queue.extend(next);
            }
            Err(error) => state.listing_failed(&directory, error),
        }
        if !state.maybe_progress(&directory) {
            break;
        }
    }
    state.finish();
}

pub(super) fn report_spawn_failure(
    tx: &Sender<ScanMessage>,
    cancel: &Arc<AtomicBool>,
    operation: &str,
    error: io::Error,
) {
    cancel.store(true, Ordering::Relaxed);
    if tx
        .send(ScanMessage::Error(format!(
            "{operation} worker could not start: {error}"
        )))
        .is_err()
    {
        return;
    }
    let _ = tx.send(ScanMessage::Done(ScanProgress {
        scanned: 0,
        bytes: 0,
        errors: 1,
        elapsed_ms: 0,
        current_path: String::new(),
    }));
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
