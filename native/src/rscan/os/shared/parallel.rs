use crate::scanner::ScanMessage;
use crate::types::{FileEntry, ScanProgress};
use crate::vfs::{BackendHandle, VfsMeta};
use crossbeam_channel::Sender;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub(super) fn run(
    backend: BackendHandle,
    root: String,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
    start: Instant,
) {
    let par = backend.parallelism().clamp(2, 16);
    let pool = match rayon::ThreadPoolBuilder::new().num_threads(par).build() {
        Ok(pool) => pool,
        Err(_) => {
            serial_fallback(backend, root, tx, cancel, start);
            return;
        }
    };

    let mut frontier = vec![(root.clone(), 1u32)];
    let mut batch: Vec<FileEntry> = Vec::with_capacity(super::BATCH);
    let mut last_progress = Instant::now();
    let mut scanned = 0u64;
    let mut bytes = 0u64;
    let mut errors = 0u64;
    let mut failed: Vec<(String, String)> = Vec::new();

    while !frontier.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let level: Vec<_> = pool.install(|| {
            frontier
                .par_iter()
                .map(|(dir, depth)| {
                    if cancel.load(Ordering::Relaxed) {
                        return (dir.clone(), *depth, Ok(Vec::new()));
                    }
                    (
                        dir.clone(),
                        *depth,
                        backend.list_dir(dir).map_err(|e| e.to_string()),
                    )
                })
                .collect()
        });

        let mut next = Vec::new();
        for (dir, depth, listed) in level {
            let entries = match listed {
                Ok(entries) => entries,
                Err(e) => {
                    errors += 1;
                    if failed.len() < super::MAX_ERRORS_TRACKED {
                        failed.push((dir, format!("list_dir: {e}")));
                    }
                    continue;
                }
            };
            emit_entries(
                &dir,
                depth,
                entries,
                &mut next,
                &mut batch,
                &tx,
                &mut scanned,
                &mut bytes,
                &cancel,
            );
            if last_progress.elapsed().as_millis() > super::PROGRESS_MS {
                send_progress(&tx, start, scanned, bytes, errors, &dir);
                last_progress = Instant::now();
            }
        }
        frontier = next;
    }

    if !batch.is_empty() {
        let _ = tx.send(ScanMessage::Entries(batch));
    }
    if !failed.is_empty() {
        let _ = tx.send(ScanMessage::FailedPaths(failed));
    }
    let _ = tx.send(ScanMessage::Done(ScanProgress {
        scanned,
        bytes,
        errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: String::new(),
    }));
}

#[allow(clippy::too_many_arguments)]
fn emit_entries(
    dir: &str,
    depth: u32,
    entries: Vec<VfsMeta>,
    next: &mut Vec<(String, u32)>,
    batch: &mut Vec<FileEntry>,
    tx: &Sender<ScanMessage>,
    scanned: &mut u64,
    bytes: &mut u64,
    cancel: &AtomicBool,
) {
    let parent_arc: Arc<str> = Arc::from(dir);
    for m in entries {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let path = super::join(dir, &m.name);
        let recurse = m.is_dir && !m.is_symlink;
        let fe = FileEntry {
            path: Arc::from(path.as_str()),
            parent: parent_arc.clone(),
            name: Arc::from(m.name.as_str()),
            ext: Arc::from(super::ext_of(&m.name, m.is_dir).as_str()),
            size: m.size,
            mtime_ms: m.mtime_ms,
            btime_ms: m.btime_ms,
            is_dir: m.is_dir,
            is_symlink: m.is_symlink,
            hidden: m.hidden,
            system: m.system,
            depth,
            id: m.id.as_deref().map(Arc::from),
        };
        *scanned += 1;
        if !m.is_dir {
            *bytes += m.size;
        }
        if recurse {
            next.push((path, depth + 1));
        }
        batch.push(fe);
        if batch.len() >= super::BATCH {
            let _ = tx.send(ScanMessage::Entries(std::mem::take(batch)));
        }
    }
}

fn send_progress(
    tx: &Sender<ScanMessage>,
    start: Instant,
    scanned: u64,
    bytes: u64,
    errors: u64,
    current_path: &str,
) {
    let _ = tx.send(ScanMessage::Progress(ScanProgress {
        scanned,
        bytes,
        errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: current_path.to_string(),
    }));
}

fn serial_fallback(
    backend: BackendHandle,
    root: String,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
    start: Instant,
) {
    let mut frontier = vec![(root, 1u32)];
    let mut batch = Vec::with_capacity(super::BATCH);
    let (mut scanned, mut bytes, mut errors) = (0, 0, 0);
    while let Some((dir, depth)) = frontier.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match backend.list_dir(&dir) {
            Ok(entries) => emit_entries(
                &dir,
                depth,
                entries,
                &mut frontier,
                &mut batch,
                &tx,
                &mut scanned,
                &mut bytes,
                &cancel,
            ),
            Err(_) => errors += 1,
        }
    }
    if !batch.is_empty() {
        let _ = tx.send(ScanMessage::Entries(batch));
    }
    let _ = tx.send(ScanMessage::Done(ScanProgress {
        scanned,
        bytes,
        errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
        current_path: String::new(),
    }));
}
